#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::sections::resource_sdk_settings::Settings as ConfiguredSettings;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::files::records::{
    CreatePurpose, FileCreate, FileDeleted, FileKind, FileObject, FileUpload, FilesPurpose,
};
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;
use uuid::Uuid;

use crate::api::file_http_upload::service::Mode;
use crate::api::resource_sdk_settings::ResourceSdkSettings;
use crate::api::resource_sdk_settings::timeout::FineTuningTimeout;

use super::service::FileWrite;
use super::{
    ConfigDelete, DeleteInput, DeleteLogger, DeleteParams, DeleteTarget, FileWriteError,
    FileWriteProviders, LoggerSeed, ProviderCall, SuppliedClient, UploadInput, UploadTarget,
    WriteParams,
};

const CALL_ID: &str = "00000000-0000-4000-8000-000000000001";

type UploadRequest = UploadInput<String, &'static str>;
type DeleteRequest = DeleteInput<String, &'static str>;
type Outcome<Value> = Result<Value, FileWriteError<Rejected>>;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("the provider answered {status}: {message}")]
struct Rejected {
    status: u16,
    message: String,
}

#[derive(Debug, Clone, PartialEq)]
struct Credentials {
    mode: Mode,
    api_base: Option<String>,
    api_key: Option<String>,
    api_version: Option<String>,
    organization: Option<String>,
    timeout: FineTuningTimeout,
    max_retries: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
enum Logged {
    Existing(String),
    Created(LoggerSeed),
}

#[derive(Debug, Clone, PartialEq)]
enum Seen {
    ConfigUpload {
        config: ProviderId,
        mode: Mode,
        data: FileCreate,
        headers: IndexMap<String, String>,
        api_base: Option<String>,
        api_key: Option<String>,
        logger: String,
        client: Option<SuppliedClient<&'static str>>,
        timeout: FineTuningTimeout,
    },
    OpenAiUpload(Credentials, FileCreate),
    AzureUpload(Credentials, FileCreate),
    OpenAiDelete(Credentials, String),
    AzureDelete {
        call: Credentials,
        file_id: String,
        client: Option<SuppliedClient<&'static str>>,
        api_key: Option<String>,
        api_base: Option<String>,
    },
    ConfigDelete {
        config: ProviderId,
        mode: Mode,
        file_id: String,
        api_key: Option<String>,
        api_base: Option<String>,
        trusted: Option<Vec<String>>,
        headers: IndexMap<String, String>,
        logger: Logged,
        client: Option<SuppliedClient<&'static str>>,
        timeout: FineTuningTimeout,
    },
}

#[derive(Clone, Default)]
struct Recorder {
    seen: Arc<Mutex<Vec<Seen>>>,
    failure: Option<Rejected>,
}

impl Recorder {
    fn seen(&self) -> Vec<Seen> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn push(&self, seen: Seen) {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(seen);
    }
}

fn exposed(key: Option<&SecretString>) -> Option<String> {
    key.map(|secret| secret.expose_secret().to_owned())
}

fn credentials(call: &ProviderCall) -> Credentials {
    Credentials {
        mode: call.mode,
        api_base: call.api_base.clone(),
        api_key: exposed(call.api_key.as_ref()),
        api_version: call.api_version.clone(),
        organization: call.organization.clone(),
        timeout: call.timeout,
        max_retries: call.max_retries,
    }
}

fn file_object() -> FileObject {
    FileObject {
        id: "file-abc".to_owned(),
        bytes: 3,
        created_at: 1_700_000_000,
        filename: "upload".to_owned(),
        object: FileKind::File,
        purpose: FilesPurpose::Batch,
        status: None,
        expires_at: None,
        status_details: None,
    }
}

// the target a recorder answers, from either write function
enum Target<'call> {
    Upload(UploadTarget<'call, Recorder>),
    Delete(DeleteTarget<'call, Recorder>),
}

fn seen(target: Target<'_>) -> Seen {
    match target {
        Target::Upload(UploadTarget::Config(upload)) => Seen::ConfigUpload {
            config: upload.config,
            mode: upload.mode,
            data: upload.data,
            headers: upload.headers,
            api_base: upload.api_base.map(str::to_owned),
            api_key: exposed(upload.api_key),
            logger: upload.logger.clone(),
            client: upload.client.cloned(),
            timeout: upload.timeout,
        },
        Target::Upload(UploadTarget::OpenAi { call, data }) => {
            Seen::OpenAiUpload(credentials(&call), data)
        }
        Target::Upload(UploadTarget::Azure { call, data, .. }) => {
            Seen::AzureUpload(credentials(&call), data)
        }
        Target::Delete(DeleteTarget::Config(delete)) => config_delete_seen(delete),
        Target::Delete(DeleteTarget::OpenAi { call, file_id }) => {
            Seen::OpenAiDelete(credentials(&call), file_id.to_owned())
        }
        Target::Delete(DeleteTarget::Azure {
            call,
            file_id,
            client,
            params,
        }) => seen_azure_delete(&call, file_id, client, params),
    }
}

fn seen_azure_delete(
    call: &ProviderCall,
    file_id: &str,
    client: Option<&SuppliedClient<&'static str>>,
    params: DeleteParams<'_>,
) -> Seen {
    Seen::AzureDelete {
        call: credentials(call),
        file_id: file_id.to_owned(),
        client: client.cloned(),
        api_key: exposed(params.api_key),
        api_base: params.api_base.map(str::to_owned),
    }
}

fn config_delete_seen(delete: ConfigDelete<'_, Recorder>) -> Seen {
    Seen::ConfigDelete {
        config: delete.config,
        mode: delete.mode,
        file_id: delete.file_id.to_owned(),
        api_key: exposed(delete.params.api_key),
        api_base: delete.params.api_base.map(str::to_owned),
        trusted: delete
            .params
            .credentials
            .map(|trusted| trusted.values.keys().cloned().collect()),
        headers: delete.headers,
        logger: match delete.logger {
            DeleteLogger::Existing(logger) => Logged::Existing(logger.clone()),
            DeleteLogger::Created(seed) => Logged::Created(seed),
        },
        client: delete.client.cloned(),
        timeout: delete.timeout,
    }
}

impl FileWriteProviders for Recorder {
    type Config = ProviderId;
    type Logger = String;
    type Client = &'static str;
    type Error = Rejected;

    // the providers LiteLLM's get_provider_files_config answers a config for
    fn files_config(&self, provider: ProviderId) -> Option<ProviderId> {
        matches!(
            provider,
            ProviderId::Gemini
                | ProviderId::VertexAi
                | ProviderId::Bedrock
                | ProviderId::Manus
                | ProviderId::Anthropic
                | ProviderId::Mistral
                | ProviderId::Xai
        )
        .then_some(provider)
    }

    fn upload(
        &self,
        target: UploadTarget<'_, Self>,
    ) -> impl Future<Output = Result<FileObject, Rejected>> + Send {
        self.push(seen(Target::Upload(target)));
        ready(self.failure.clone().map_or_else(|| Ok(file_object()), Err))
    }

    fn delete(
        &self,
        target: DeleteTarget<'_, Self>,
    ) -> impl Future<Output = Result<FileDeleted, Rejected>> + Send {
        let file_id = match &target {
            DeleteTarget::OpenAi { file_id, .. } | DeleteTarget::Azure { file_id, .. } => {
                (*file_id).to_owned()
            }
            DeleteTarget::Config(delete) => delete.file_id.to_owned(),
        };
        self.push(seen(Target::Delete(target)));
        let deleted = FileDeleted {
            id: file_id,
            object: FileKind::File,
            deleted: false,
        };
        ready(self.failure.clone().map_or_else(|| Ok(deleted), Err))
    }
}

struct Resolution {
    catalog: ModelCatalog,
    names: BTreeSet<String>,
    environment: BTreeMap<String, String>,
}

impl Resolution {
    fn new() -> Self {
        Self {
            catalog: ModelCatalog::parse(b"{}", UnknownFields::Reject)
                .expect("an empty catalog parses"),
            names: BTreeSet::new(),
            environment: BTreeMap::new(),
        }
    }

    fn context(&self) -> ResolutionContext<'_> {
        ResolutionContext {
            catalog: &self.catalog,
            known_model_names: &self.names,
            custom_providers: &self.names,
            environment: &self.environment,
            registry: JSONProviderRegistry::default(),
            policies: ProviderPolicies::default(),
            globals: GlobalCredentials::default(),
        }
    }
}

fn started() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").expect("a fixed instant")
}

fn call_id() -> Uuid {
    Uuid::parse_str(CALL_ID).expect("a fixed call id")
}

fn settings(configured: &ConfiguredSettings) -> ResourceSdkSettings {
    ResourceSdkSettings::from(configured)
}

fn params() -> WriteParams<String, &'static str> {
    WriteParams {
        api_base: None,
        api_key: None,
        api_version: None,
        organization: None,
        timeout: None,
        request_timeout: None,
        max_retries: None,
        passthrough: None,
        litellm_call_id: None,
        id: None,
        logger: Some("caller-logger".to_owned()),
        client: None,
        credentials: None,
    }
}

// which of the two write functions a test builds its input for
#[derive(Debug, Clone, Copy)]
enum Operation {
    Upload,
    Delete,
}

// the input a test runs, with the provider it names or the null it sends as given
enum Request {
    Upload(UploadRequest),
    Delete(DeleteRequest),
}

// the record the provider answered with, before the test reads the branch it asked for
enum Answer {
    File(Outcome<FileObject>),
    Deleted(Outcome<FileDeleted>),
}

impl Answer {
    fn file(self) -> Outcome<FileObject> {
        match self {
            Self::File(outcome) => outcome,
            Self::Deleted(_) => panic!("the case uploads"),
        }
    }

    fn deleted(self) -> Outcome<FileDeleted> {
        match self {
            Self::File(_) => panic!("the case deletes"),
            Self::Deleted(outcome) => outcome,
        }
    }
}

// a case field that tells an absent key from an explicit null from a value
#[derive(Debug, Clone, Copy)]
enum Presence<'value> {
    Absent,
    Null,
    Value(&'value str),
}

fn request(operation: Operation, provider: Presence<'_>) -> Request {
    let named = match provider {
        Presence::Absent => None,
        Presence::Null => Some(None),
        Presence::Value(name) => Some(Some(name.to_owned())),
    };
    match operation {
        Operation::Upload => Request::Upload(UploadInput {
            file: FileUpload {
                filename: None,
                content_type: None,
                data: b"abc".to_vec(),
            },
            purpose: CreatePurpose::Batch,
            expires_after: None,
            custom_llm_provider: named,
            extra_headers: None,
            extra_body: None,
            params: params(),
        }),
        Operation::Delete => Request::Delete(DeleteInput {
            file_id: "file-abc".to_owned(),
            model: None,
            custom_llm_provider: named.flatten(),
            extra_headers: None,
            extra_body: None,
            params: params(),
        }),
    }
}

fn upload_input(provider: Presence<'_>) -> UploadRequest {
    let Request::Upload(input) = request(Operation::Upload, provider) else {
        panic!("the case uploads");
    };
    input
}

fn delete_input(provider: Option<&str>) -> DeleteRequest {
    let named = provider.map_or(Presence::Absent, Presence::Value);
    let Request::Delete(input) = request(Operation::Delete, named) else {
        panic!("the case deletes");
    };
    input
}

fn write_with(
    recorder: &Recorder,
    configured: &ConfiguredSettings,
    mode: Mode,
    request: Request,
) -> Answer {
    let resolution = Resolution::new();
    let context = resolution.context();
    let service = FileWrite {
        providers: recorder.clone(),
        settings: settings(configured),
        resolution: &context,
        clock: started,
        call_ids: call_id,
    };
    futures_executor::block_on(async {
        match request {
            Request::Upload(input) => Answer::File(service.create_file(mode, &input).await),
            Request::Delete(input) => Answer::Deleted(service.delete_file(mode, &input).await),
        }
    })
}

fn upload(mode: Mode, input: UploadRequest) -> (Outcome<FileObject>, Vec<Seen>) {
    let recorder = Recorder::default();
    let configured = ConfiguredSettings::default();
    let answer = write_with(&recorder, &configured, mode, Request::Upload(input));
    (answer.file(), recorder.seen())
}

fn delete(mode: Mode, input: DeleteRequest) -> (Outcome<FileDeleted>, Vec<Seen>) {
    let recorder = Recorder::default();
    let configured = ConfiguredSettings::default();
    let answer = write_with(&recorder, &configured, mode, Request::Delete(input));
    (answer.deleted(), recorder.seen())
}

fn sent_data(seen: &[Seen]) -> FileCreate {
    match seen {
        [Seen::OpenAiUpload(_, data) | Seen::AzureUpload(_, data)] => data.clone(),
        [Seen::ConfigUpload { data, .. }] => data.clone(),
        other => panic!("expected one upload, saw {other:?}"),
    }
}

fn sent_call(seen: &[Seen]) -> Credentials {
    match seen {
        [Seen::OpenAiUpload(call, _) | Seen::AzureUpload(call, _)] => call.clone(),
        [Seen::OpenAiDelete(call, _) | Seen::AzureDelete { call, .. }] => call.clone(),
        other => panic!("expected one sdk call, saw {other:?}"),
    }
}

fn sent_timeout(seen: &[Seen]) -> FineTuningTimeout {
    match seen {
        [Seen::ConfigUpload { timeout, .. } | Seen::ConfigDelete { timeout, .. }] => *timeout,
        calls => sent_call(calls).timeout,
    }
}

mod dispatch;
mod guards;
