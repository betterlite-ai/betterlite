#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_azure::endpoints::projection::LiteLlmParams;
use betterlite_config::sections::resource_sdk_settings::Settings;
use betterlite_error::error::ErrorClass;
use betterlite_http::error::HttpError;
use betterlite_logging::fields::payload::StandardLoggingPayload;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::controls::call_params::TrustedCredentials;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::json_provider_registry::JSONProviderRegistry;
use betterlite_models::provider_defaults_a::policies::ProviderPolicies;
use betterlite_models::provider_defaults_b::globals::GlobalCredentials;
use betterlite_models::resolution::ResolutionContext;
use betterlite_openai::files::error::FilesError;
use betterlite_openai::files::records::{BinaryContent, FileContentRequest};
use betterlite_openai::files::stream::FileContentStreamingResult;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use futures_util::StreamExt as _;
use futures_util::stream;
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;
use uuid::Uuid;

use crate::api::file_stream_logging::logging::{
    CallbackFailure, FailureCall, FileStreamLogger, SuccessCall,
};
use crate::api::file_stream_logging::source::SourceFailure;
use crate::api::fine_tuning_read::{ReadStructuredTimeout, ReadTimeout, ReadTimeoutValue};
use crate::api::resource_sdk_settings::timeout::{FineTuningTimeout, StructuredTimeout};
use crate::api::vector_store_http_write::service::{ClientKind, SuppliedClient};

use super::error::FileContentError;
use super::service::Selection;
use super::streaming::source_failure as stream_source_failure;
use super::target::{
    CallerClient, ContentLogger, ContentLogging, ContentMode, ContentOperation, ContentReply,
    ContentTarget, FileContentDispatch, GenericContent, NewLogger, StreamIdentity, SuppliedSdk,
};
use super::{
    ContentAnswer, ContentCall, ContentContext, ContentEntry, ContentKwargs, FileContentStream,
    StreamedContent,
};

const STARTED: &str = "2026-01-01T00:00:00+00:00";
const GENERATED_ID: &str = "00000000-0000-0000-0000-000000000001";

fn started() -> DateTime<FixedOffset> {
    DateTime::<FixedOffset>::parse_from_rfc3339(STARTED).unwrap()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FakeConfig(ProviderId);

#[derive(Debug, Clone, PartialEq)]
struct GenericSeen {
    config: FakeConfig,
    api_key: Option<String>,
    api_base: Option<String>,
    trusted: Option<Vec<String>>,
    headers: BTreeMap<String, String>,
    created: Option<NewLogger>,
    client: Option<ClientKind>,
    mode: ContentMode,
}

#[derive(Debug, Clone, PartialEq)]
enum Seen {
    Compatible {
        api_base: String,
        api_key: Option<String>,
        organization: Option<String>,
        max_retries: Option<i64>,
        mode: ContentMode,
        client: Option<SuppliedSdk>,
    },
    Azure {
        api_base: Option<String>,
        api_key: Option<String>,
        api_version: Option<String>,
        max_retries: Option<i64>,
        mode: ContentMode,
        client: Option<SuppliedSdk>,
    },
    Generic(Box<GenericSeen>),
    XaiBatch {
        batch_id: String,
        api_base: Option<String>,
        api_key: Option<String>,
        timeout: FineTuningTimeout,
    },
}

impl Seen {
    fn mode(&self) -> ContentMode {
        match self {
            Self::Compatible { mode, .. } | Self::Azure { mode, .. } => *mode,
            Self::Generic(generic) => generic.mode,
            Self::XaiBatch { timeout, .. } => ContentMode {
                is_async: false,
                timeout: *timeout,
            },
        }
    }

    fn generic(&self) -> GenericSeen {
        match self {
            Self::Generic(generic) => (**generic).clone(),
            Self::Compatible { .. } | Self::Azure { .. } | Self::XaiBatch { .. } => {
                panic!("not a generic content call: {self:?}")
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Sent {
    target: Seen,
    request: FileContentRequest,
    chunk_size: Option<NonZeroUsize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("provider status {0}")]
struct FakeFailure(u16);

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecordedIdentity {
    model: String,
    custom_llm_provider: String,
    api_base: Option<String>,
}

// The logging object file_content_streaming annotates and the wrapper logs through
#[derive(Default)]
struct RecordingLogging {
    identities: Mutex<Vec<RecordedIdentity>>,
    successes: AtomicUsize,
    failures: AtomicUsize,
    times: Mutex<Vec<(DateTime<FixedOffset>, DateTime<FixedOffset>)>>,
}

impl RecordingLogging {
    fn identities(&self) -> Vec<RecordedIdentity> {
        self.identities.lock().unwrap().clone()
    }

    fn successes(&self) -> usize {
        self.successes.load(Ordering::SeqCst)
    }

    fn failures(&self) -> usize {
        self.failures.load(Ordering::SeqCst)
    }

    fn success_times(&self) -> Vec<(DateTime<FixedOffset>, DateTime<FixedOffset>)> {
        self.times.lock().unwrap().clone()
    }
}

impl ContentLogging for RecordingLogging {
    fn record_identity(&self, identity: &StreamIdentity<'_>) {
        self.identities.lock().unwrap().push(RecordedIdentity {
            model: identity.model.to_owned(),
            custom_llm_provider: identity.custom_llm_provider.to_owned(),
            api_base: identity.api_base.map(str::to_owned),
        });
    }

    fn stream_logger(self: Arc<Self>) -> Arc<dyn FileStreamLogger> {
        self
    }
}

#[async_trait]
impl FileStreamLogger for RecordingLogging {
    fn start_time(&self) -> Option<DateTime<FixedOffset>> {
        None
    }

    fn details_api_base(&self) -> Option<String> {
        self.identities()
            .last()
            .and_then(|last| last.api_base.clone())
    }

    fn build_payload(
        &self,
        _response: &Json,
        _start_time: DateTime<FixedOffset>,
        _end_time: DateTime<FixedOffset>,
    ) -> Option<StandardLoggingPayload> {
        None
    }

    fn success_handler(&self, call: SuccessCall) -> Result<(), CallbackFailure> {
        self.times
            .lock()
            .map_err(|source| CallbackFailure {
                message: source.to_string(),
            })?
            .push((call.start_time, call.end_time));
        self.successes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn async_success_handler(&self, call: SuccessCall) -> Result<(), CallbackFailure> {
        self.success_handler(call)
    }

    fn success_sync_callbacks(&self, _call: SuccessCall) -> Result<(), CallbackFailure> {
        Ok(())
    }

    fn failure_handler(&self, _call: FailureCall) -> Result<(), CallbackFailure> {
        self.failures.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn async_failure_handler(&self, _call: FailureCall) -> Result<(), CallbackFailure> {
        Ok(())
    }
}

struct FakeDispatch {
    azure_params: Mutex<Vec<LiteLlmParams>>,
    configured: Vec<ProviderId>,
    sent: Mutex<Vec<Sent>>,
    created: Mutex<Vec<NewLogger>>,
    loggers: Mutex<Vec<Arc<RecordingLogging>>>,
    failure: Option<FakeFailure>,
    wrong_reply: bool,
}

impl FileContentDispatch for FakeDispatch {
    type Config = FakeConfig;
    type Client = ();
    type Logging = RecordingLogging;
    type Error = FakeFailure;

    fn files_config(&self, provider: ProviderId) -> Option<FakeConfig> {
        self.configured
            .contains(&provider)
            .then_some(FakeConfig(provider))
    }

    fn create_logging(&self, seed: &NewLogger) -> Arc<RecordingLogging> {
        let logger = Arc::new(RecordingLogging::default());
        self.created.lock().unwrap().push(seed.clone());
        self.loggers.lock().unwrap().push(Arc::clone(&logger));
        logger
    }

    fn content(
        &self,
        target: ContentTarget<'_, FakeConfig, ()>,
        operation: ContentOperation<'_>,
    ) -> impl Future<Output = Result<ContentReply, FakeFailure>> + Send {
        if let ContentTarget::Azure(azure) = &target {
            self.azure_params.lock().unwrap().push(azure.params.clone());
        }
        let (request, chunk_size, streamed) = match operation {
            ContentOperation::Buffered { request } => (request.clone(), None, false),
            ContentOperation::Streamed {
                request,
                chunk_size,
            } => (request.clone(), chunk_size, true),
        };
        self.sent.lock().unwrap().push(Sent {
            target: seen(target),
            request,
            chunk_size,
        });
        let reply = if streamed == self.wrong_reply {
            ContentReply::Buffered(Box::new(binary()))
        } else {
            ContentReply::Streamed(Box::new(stream_result()))
        };
        self.failure
            .map_or_else(|| ready(Ok(reply)), |failure| ready(Err(failure)))
    }
}

fn seen(target: ContentTarget<'_, FakeConfig, ()>) -> Seen {
    match target {
        ContentTarget::Compatible(content) => Seen::Compatible {
            api_key: content
                .api_key
                .map(|secret| secret.expose_secret().to_owned()),
            api_base: content.api_base,
            organization: content.organization,
            max_retries: content.max_retries,
            mode: content.mode,
            client: content.client,
        },
        ContentTarget::Azure(content) => Seen::Azure {
            api_key: content
                .api_key
                .map(|secret| secret.expose_secret().to_owned()),
            api_base: content.api_base,
            api_version: content.api_version,
            max_retries: content.max_retries,
            mode: content.mode,
            client: content.client,
        },
        ContentTarget::Generic(boxed) => Seen::Generic(Box::new(generic_snapshot(*boxed))),
        ContentTarget::XaiBatch(content) => Seen::XaiBatch {
            batch_id: content.batch_id.to_owned(),
            api_base: content.api_base.map(str::to_owned),
            api_key: content
                .api_key
                .map(|secret| secret.expose_secret().to_owned()),
            timeout: content.timeout,
        },
    }
}

fn generic_snapshot(content: GenericContent<'_, FakeConfig, ()>) -> GenericSeen {
    let GenericContent {
        config,
        params,
        trusted,
        headers,
        logger,
        client,
        mode,
    } = content;
    GenericSeen {
        config,
        api_key: params
            .api_key
            .map(|secret| secret.expose_secret().to_owned()),
        api_base: params.api_base,
        trusted: trusted.map(|snapshot| snapshot.values.keys().cloned().collect()),
        headers,
        created: match logger {
            ContentLogger::Created(created) => Some(*created),
            ContentLogger::Supplied(_) => None,
        },
        client: client.map(|supplied| supplied.kind),
        mode,
    }
}

fn binary() -> BinaryContent {
    BinaryContent {
        status: 200,
        headers: IndexMap::from([(
            String::from("content-type"),
            String::from("application/octet-stream"),
        )]),
        body: vec![0, 255],
    }
}

fn stream_result() -> FileContentStreamingResult {
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-file"),
        HeaderValue::from_static("capture"),
    );
    FileContentStreamingResult::new(
        stream::iter(vec![Ok::<Vec<u8>, HttpError>(vec![0, 255])]).boxed(),
        &headers,
        None,
    )
}

// every model, catalog and environment value the resolver may read, held for the test's lifetime
#[derive(Default)]
struct Env {
    catalog: ModelCatalog,
    names: BTreeSet<String>,
    environment: BTreeMap<String, String>,
}

impl Env {
    fn resolution(&self) -> ResolutionContext<'_> {
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

fn service<'deps>(
    resolution: &'deps ResolutionContext<'deps>,
    settings: Settings,
    configured: &[ProviderId],
) -> FileContentStream<'deps, FakeDispatch> {
    FileContentStream {
        dispatch: FakeDispatch {
            azure_params: Mutex::new(Vec::new()),
            configured: configured.to_vec(),
            sent: Mutex::new(Vec::new()),
            created: Mutex::new(Vec::new()),
            loggers: Mutex::new(Vec::new()),
            failure: None,
            wrong_reply: false,
        },
        settings,
        resolution,
        clock: || DateTime::<FixedOffset>::parse_from_rfc3339(STARTED).unwrap(),
        call_ids: || Uuid::from_u128(1),
    }
}

// every Settings field, named here once so a test sets only the ones it reads
const fn default_settings() -> Settings {
    Settings {
        api_base: None,
        api_version: None,
        api_key: None,
        openai_key: None,
        azure_key: None,
        azure_ad_token: None,
        organization: None,
        vertex_project: None,
        vertex_location: None,
        openai_base_url: None,
        openai_api_base: None,
        openai_api_key: None,
        openai_organization: None,
        azure_api_base: None,
        azure_api_version: None,
        azure_openai_api_key: None,
        azure_api_key: None,
        vertexai_project: None,
        vertexai_location: None,
        vertexai_credentials: None,
    }
}

fn reader<'deps>(
    resolution: &'deps ResolutionContext<'deps>,
) -> FileContentStream<'deps, FakeDispatch> {
    service(
        resolution,
        Settings::default(),
        &[
            ProviderId::Anthropic,
            ProviderId::Bedrock,
            ProviderId::VertexAi,
        ],
    )
}

fn content_call(kwargs: &ContentKwargs) -> ContentCall<'_> {
    ContentCall {
        file_id: "file-1",
        model: None,
        custom_llm_provider: None,
        extra_headers: None,
        extra_body: None,
        chunk_size: 1024 * 1024,
        stream: false,
        kwargs,
    }
}

const fn bare() -> ContentContext<'static, (), RecordingLogging> {
    ContentContext {
        trusted: None,
        client: None,
        logger: None,
        logging: None,
    }
}

const fn sdk_context(kind: SuppliedSdk) -> ContentContext<'static, (), RecordingLogging> {
    ContentContext {
        trusted: None,
        client: Some(CallerClient::Sdk(kind)),
        logger: None,
        logging: None,
    }
}

const fn http_context(
    kind: ClientKind,
    trusted: Option<&TrustedCredentials>,
) -> ContentContext<'_, (), RecordingLogging> {
    ContentContext {
        trusted,
        client: Some(CallerClient::Http(SuppliedClient {
            kind,
            transport: &(),
        })),
        logger: None,
        logging: None,
    }
}

fn logging_context(logger: &Arc<RecordingLogging>) -> ContentContext<'_, (), RecordingLogging> {
    ContentContext {
        trusted: None,
        client: None,
        logger: None,
        logging: Some(Arc::clone(logger)),
    }
}

fn sent(read: &FileContentStream<'_, FakeDispatch>) -> Vec<Sent> {
    read.dispatch.sent.lock().unwrap().clone()
}

async fn run<'call>(
    read: &FileContentStream<'_, FakeDispatch>,
    entry: ContentEntry,
    call: &ContentCall<'call>,
    context: ContentContext<'call, (), RecordingLogging>,
) -> Result<ContentAnswer, FileContentError<FakeFailure>> {
    read.run(entry, call, context).await
}

fn answered_buffered(answer: ContentAnswer) -> BinaryContent {
    match answer {
        ContentAnswer::Buffered(content) => *content,
        ContentAnswer::Streamed(_) => panic!("not a buffered answer"),
    }
}

fn answered_streamed(answer: ContentAnswer) -> StreamedContent {
    match answer {
        ContentAnswer::Streamed(content) => *content,
        ContentAnswer::Buffered(_) => panic!("not a streamed answer"),
    }
}

async fn drained(content: &mut StreamedContent) -> Vec<Vec<u8>> {
    let mut iter = content.response.async_iter().unwrap();
    let mut chunks = Vec::new();
    while let Some(chunk) = iter.next_chunk().await {
        chunks.push(chunk.unwrap());
    }
    chunks
}

fn text_map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

mod azure;
mod defaults;
mod dispatch;
mod streaming;
mod unsupported;
