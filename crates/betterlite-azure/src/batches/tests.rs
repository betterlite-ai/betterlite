#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::sections::azure_endpoints::Settings as EndpointSettings;
use betterlite_http::client_kind::ClientKind;
use betterlite_openai::batches::error::SdkError;
use betterlite_openai::batches::records::{BatchCreate, BatchObject, BatchPage, BatchQuery};
use betterlite_openai::batches::sdk::BatchesSdk;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::runtime::{Builder, Runtime};

use super::client::{ClientMaterializer, ClientSettings, Connection};
use super::error::AzureBatchesError;
use super::{Operation, Reply, call};
use crate::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use crate::azure_urls::EndpointSelection;
use crate::credentials::cache::RefreshProviderCache;
use crate::credentials::source::{CallableTokenProvider, CredentialSources, SourceFailure};
use crate::endpoints::get_client::ClientCache;
use crate::endpoints::projection::{
    ClientIdentity, ClientInputs, ClientKind as OutcomeKind, ClientOutcome, ClientSource,
    LiteLlmParams, SessionKind, SuppliedClient, V1Key,
};
use crate::oidc_exchange::OidcExchangeInput;

const MISSING: &str = "OpenAI client is not initialized. Make sure api_key is passed or OPENAI_API_KEY is set in the environment.";
const NOT_ASYNC: &str =
    "OpenAI client is not an instance of AsyncOpenAI. Make sure you passed an AsyncOpenAI client.";
const CANCEL_NOT_ASYNC: &str = "Azure client is not an instance of AsyncAzureOpenAI or AsyncOpenAI. Make sure you passed an async client.";
const NOT_SYNC: &str =
    "Azure client is not an instance of AzureOpenAI or OpenAI. Make sure you passed a sync client.";

pub(super) type Calls = Arc<Mutex<Vec<Recorded>>>;
pub(super) type Shared = Arc<Mutex<Vec<Projection>>>;

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Recorded {
    Create(BatchCreate),
    Retrieve(BatchQuery),
    Cancel(BatchQuery),
    List {
        after: Option<String>,
        limit: Option<i64>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Failure {
    pub(super) status: u16,
    pub(super) error_type: Option<String>,
    pub(super) param: Option<String>,
    pub(super) code: Option<String>,
    pub(super) message: String,
}

impl Failure {
    fn error(&self) -> SdkError {
        SdkError::Status {
            status: self.status,
            error_type: self.error_type.clone(),
            param: self.param.clone(),
            code: self.code.clone(),
            message: self.message.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Answer {
    Value(String),
    Failure(Failure),
    // a non-status SDK failure, which the handler hands on unchanged
    Timeout { seconds: f64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    Built,
    Missing,
}

pub(super) struct Sdk {
    kind: ClientKind,
    calls: Calls,
    answer: Answer,
}

impl Sdk {
    fn answered(&self, call: Recorded) -> Result<Vec<u8>, SdkError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        match &self.answer {
            Answer::Value(body) => Ok(body.clone().into_bytes()),
            Answer::Failure(failure) => Err(failure.error()),
            Answer::Timeout { seconds } => Err(SdkError::InvalidTimeout { seconds: *seconds }),
        }
    }

    fn answered_page(&self, call: Recorded) -> Result<BatchPage, SdkError> {
        let bytes = self.answered(call)?;
        serde_json::from_slice(&bytes).map_err(|source| SdkError::Body { source })
    }
}

impl BatchesSdk for Sdk {
    type Page = BatchPage;

    fn kind(&self) -> ClientKind {
        self.kind
    }

    fn create(
        &self,
        request: &BatchCreate,
    ) -> impl Future<Output = Result<Vec<u8>, SdkError>> + Send {
        ready(self.answered(Recorded::Create(request.clone())))
    }

    fn retrieve(
        &self,
        request: &BatchQuery,
    ) -> impl Future<Output = Result<Vec<u8>, SdkError>> + Send {
        ready(self.answered(Recorded::Retrieve(request.clone())))
    }

    fn cancel(
        &self,
        request: &BatchQuery,
    ) -> impl Future<Output = Result<Vec<u8>, SdkError>> + Send {
        ready(self.answered(Recorded::Cancel(request.clone())))
    }

    fn list(
        &self,
        after: Option<&str>,
        limit: Option<i64>,
    ) -> impl Future<Output = Result<BatchPage, SdkError>> + Send {
        let call = Recorded::List {
            after: after.map(str::to_owned),
            limit,
        };
        ready(self.answered_page(call))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct Projection {
    pub(super) class: &'static str,
    pub(super) supplied: bool,
    pub(super) api_key: Option<String>,
    pub(super) base_url: Option<String>,
    pub(super) api_version: Option<String>,
    pub(super) timeout: Option<f64>,
    pub(super) max_retries: Option<i64>,
    pub(super) ad_token: Option<String>,
}

pub(super) struct Materializer {
    mode: Mode,
    answer: Answer,
    calls: Calls,
    seen: Shared,
}

impl ClientMaterializer for Materializer {
    type Client = Sdk;

    fn materialize(&self, outcome: &ClientOutcome<'_>) -> Result<Option<Sdk>, SdkError> {
        if self.mode == Mode::Missing {
            return Ok(None);
        }
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(projection(outcome));
        Ok(Some(Sdk {
            kind: sync_kind(outcome.kind),
            calls: Arc::clone(&self.calls),
            answer: self.answer.clone(),
        }))
    }
}

const fn sync_kind(kind: OutcomeKind) -> ClientKind {
    match kind {
        OutcomeKind::AsyncAzureOpenAi | OutcomeKind::AsyncOpenAi => ClientKind::Asynchronous,
        OutcomeKind::AzureOpenAi | OutcomeKind::OpenAi | OutcomeKind::Other => ClientKind::Sync,
    }
}

const fn class_name(kind: OutcomeKind) -> &'static str {
    match kind {
        OutcomeKind::AzureOpenAi => "AzureOpenAI",
        OutcomeKind::AsyncAzureOpenAi => "AsyncAzureOpenAI",
        OutcomeKind::OpenAi => "OpenAI",
        OutcomeKind::AsyncOpenAi => "AsyncOpenAI",
        OutcomeKind::Other => "Other",
    }
}

fn endpoint_base(endpoint: &EndpointSelection) -> Option<String> {
    match (&endpoint.azure_endpoint, &endpoint.base_url) {
        (Some(azure), _) => Some(format!("{}/openai/", azure.trim_end_matches('/'))),
        (None, Some(path)) => Some(format!("{}/", path.trim_end_matches('/'))),
        (None, None) => None,
    }
}

fn v1_key(key: Option<&V1Key<'_>>) -> Option<String> {
    match key {
        Some(V1Key::Static(secret)) => Some(secret.expose_secret().to_owned()),
        Some(V1Key::Provider { .. }) | None => None,
    }
}

pub(super) fn projection(outcome: &ClientOutcome<'_>) -> Projection {
    let mut report = Projection {
        class: class_name(outcome.kind),
        supplied: outcome.source == ClientSource::Supplied,
        api_key: None,
        base_url: None,
        api_version: None,
        timeout: None,
        max_retries: None,
        ad_token: None,
    };
    match outcome.inputs.as_ref() {
        Some(ClientInputs::Azure(azure)) => {
            report.api_key = azure
                .api_key
                .as_ref()
                .map(|key| key.expose_secret().to_owned());
            report.ad_token = azure
                .azure_ad_token
                .as_ref()
                .map(|token| token.expose_secret().to_owned());
            report.base_url = endpoint_base(&azure.endpoint);
            report.api_version = Some(azure.api_version.clone());
            report.timeout = azure.timeout;
            report.max_retries = Some(azure.max_retries);
        }
        Some(ClientInputs::OpenAiV1(v1)) => {
            report.api_key = v1_key(v1.key.as_ref());
            report.base_url = v1.base_url.clone();
            report.timeout = v1.timeout;
            report.max_retries = Some(v1.max_retries);
        }
        None => {}
    }
    report
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Action {
    Create,
    Retrieve,
    Cancel,
    List,
}

#[derive(Debug, Clone)]
pub(super) struct Step {
    pub(super) action: Action,
    pub(super) is_async: bool,
    pub(super) supplied: Option<OutcomeKind>,
    pub(super) mode: Mode,
    pub(super) api_base: Option<String>,
    pub(super) api_version: Option<String>,
    pub(super) params: LiteLlmParams,
    pub(super) answer: Answer,
    pub(super) data: Option<BatchCreate>,
    pub(super) query: Option<BatchQuery>,
    pub(super) after: Option<String>,
    pub(super) limit: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum Raised {
    ClientMissing(String),
    NotAsync(String),
    CancelNotAsync(String),
    NotSync(String),
    Status {
        status: u16,
        error_type: Option<String>,
        param: Option<String>,
        code: Option<String>,
        message: String,
    },
    Invalid(String),
    Sdk(String),
    Endpoint(String),
}

// The reply call hands back, typed the way the caller reads it
#[derive(Debug, PartialEq)]
pub(super) enum Answered {
    Batch(Box<BatchObject>),
    Page(Box<BatchPage>),
}

#[derive(Debug, PartialEq)]
pub(super) struct Observation {
    pub(super) reply: Option<Answered>,
    pub(super) raised: Option<Raised>,
    pub(super) calls: Vec<Recorded>,
    pub(super) seen: Vec<Projection>,
}

struct Sources;

fn source_failure() -> SourceFailure {
    SourceFailure {
        class: "Unavailable".to_owned(),
        status: None,
        message: "the fixture has no credential sources".to_owned(),
    }
}

impl CredentialSources for Sources {
    fn client_secret<'source>(
        &'source self,
        _record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(source_failure())
    }

    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(source_failure())
    }

    fn sdk_refresh<'source>(
        &'source self,
        _scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(source_failure())
    }

    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Err(source_failure())
    }
}

#[derive(Default)]
struct Cache;

impl ClientCache for Cache {
    fn get(&self, _identity: &ClientIdentity) -> Option<OutcomeKind> {
        None
    }

    fn set(&mut self, _identity: ClientIdentity, _kind: OutcomeKind, _owned: bool) {}
}

pub(super) fn settings(api_version: Option<&str>) -> ClientSettings {
    ClientSettings {
        api_key: Some(SecretString::from("key-1")),
        api_base: Some("https://fixture.test".to_owned()),
        api_version: api_version.map(str::to_owned),
    }
}

pub(super) fn params(fields: &str) -> LiteLlmParams {
    serde_json::from_str(fields).expect("the litellm params parse")
}

pub(super) fn batch_body() -> String {
    concat!(
        r#"{"id":"batch_1","completion_window":"24h","created_at":1700000000,"#,
        r#""endpoint":"/v1/embeddings","input_file_id":"file-1","object":"batch","status":"validating","#,
        r#""cancelled_at":null,"cancelling_at":null,"completed_at":null,"error_file_id":null,"errors":null,"#,
        r#""expired_at":null,"expires_at":null,"failed_at":null,"finalizing_at":null,"in_progress_at":null,"#,
        r#""metadata":null,"model":null,"output_file_id":null,"request_counts":null,"usage":null}"#
    )
    .to_owned()
}

pub(super) fn page_body() -> String {
    format!(
        r#"{{"object":"list","data":[{}],"has_more":false,"first_id":"batch_1","last_id":"batch_1"}}"#,
        batch_body()
    )
}

pub(super) fn step(action: Action) -> Step {
    Step {
        action,
        is_async: false,
        supplied: None,
        mode: Mode::Built,
        api_base: Some("https://fixture.test".to_owned()),
        api_version: Some("2024-10-21".to_owned()),
        params: LiteLlmParams::default(),
        answer: Answer::Value(batch_body()),
        data: None,
        query: None,
        after: None,
        limit: None,
    }
}

pub(super) fn creating(data: &str) -> Step {
    Step {
        data: Some(serde_json::from_str(data).expect("the create data parses")),
        answer: Answer::Value(batch_body()),
        ..step(Action::Create)
    }
}

pub(super) fn listing(after: Option<&str>, limit: Option<i64>) -> Step {
    Step {
        after: after.map(str::to_owned),
        limit,
        answer: Answer::Value(page_body()),
        ..step(Action::List)
    }
}

pub(super) fn query_data(step: Step, data: &str) -> Step {
    Step {
        query: Some(serde_json::from_str(data).expect("the query data parses")),
        ..step
    }
}

pub(super) fn endpoint_step(api_base: &str) -> Step {
    Step {
        api_base: Some(api_base.to_owned()),
        ..step(Action::List)
    }
}

pub(super) fn async_(step: Step) -> Step {
    Step {
        is_async: true,
        ..step
    }
}

pub(super) fn supplying(step: Step, kind: OutcomeKind) -> Step {
    Step {
        supplied: Some(kind),
        ..step
    }
}

pub(super) fn missing(step: Step) -> Step {
    Step {
        mode: Mode::Missing,
        ..step
    }
}

pub(super) fn versioned(step: Step, api_version: Option<&str>) -> Step {
    Step {
        api_version: api_version.map(str::to_owned),
        ..step
    }
}

pub(super) fn with_params(step: Step, fields: &str) -> Step {
    Step {
        params: params(fields),
        ..step
    }
}

pub(super) fn failing(step: Step, failure: Failure) -> Step {
    Step {
        answer: Answer::Failure(failure),
        ..step
    }
}

pub(super) fn timed_out(step: Step, seconds: f64) -> Step {
    Step {
        answer: Answer::Timeout { seconds },
        ..step
    }
}

fn operation(step: &Step) -> Operation<'_> {
    match step.action {
        Action::Create => Operation::Create(
            step.data
                .as_ref()
                .expect("a create step carries create data"),
        ),
        Action::Retrieve => Operation::Retrieve(
            step.query
                .as_ref()
                .expect("a retrieve step carries a query"),
        ),
        Action::Cancel => {
            Operation::Cancel(step.query.as_ref().expect("a cancel step carries a query"))
        }
        Action::List => Operation::List {
            after: step.after.as_deref(),
            limit: step.limit,
        },
    }
}

// The fixture builds the connection, then hands it to the work the case asks for
fn with_connection<T>(
    step: &Step,
    materializer: &Materializer,
    work: fn(&Runtime, Connection<'_, '_, Materializer>, &Step) -> T,
) -> T {
    let endpoints = EndpointSettings::default();
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let sources = Sources;
    let mut cache = Cache;
    let mut refresh = RefreshProviderCache::new(&sources);
    let runtime = Builder::new_current_thread()
        .build()
        .expect("the test runtime builds");
    let connection = Connection {
        settings: ClientSettings {
            api_base: step.api_base.clone(),
            ..settings(step.api_version.as_deref())
        },
        params: step.params.clone(),
        provider: None,
        supplied: step.supplied.map(|kind| SuppliedClient {
            kind,
            custom_query: None,
        }),
        is_async: step.is_async,
        endpoints: &endpoints,
        environment: &environment,
        sources: &sources,
        session: SessionKind::Dedicated,
        refresh_enabled: false,
        cache: &mut cache,
        refresh: &mut refresh,
        materializer,
    };
    work(&runtime, connection, step)
}

fn call_through(
    runtime: &Runtime,
    connection: Connection<'_, '_, Materializer>,
    step: &Step,
) -> Result<Reply, AzureBatchesError> {
    let asked = operation(step);
    let client = connection.open(asked.check())?;
    runtime.block_on(call(&client, asked))
}

fn project_through(
    _runtime: &Runtime,
    mut connection: Connection<'_, '_, Materializer>,
    _step: &Step,
) -> Result<Projection, Raised> {
    let outcome = connection
        .get_openai_client()
        .map_err(|error| raised(&error))?;
    Ok(projection(&outcome))
}

fn raised(error: &AzureBatchesError) -> Raised {
    match error {
        AzureBatchesError::ClientMissing => Raised::ClientMissing(error.to_string()),
        AzureBatchesError::NotAsync => Raised::NotAsync(error.to_string()),
        AzureBatchesError::CancelNotAsync => Raised::CancelNotAsync(error.to_string()),
        AzureBatchesError::NotSync => Raised::NotSync(error.to_string()),
        AzureBatchesError::Sdk {
            source:
                SdkError::Status {
                    status,
                    error_type,
                    param,
                    code,
                    message,
                },
        } => Raised::Status {
            status: *status,
            error_type: error_type.clone(),
            param: param.clone(),
            code: code.clone(),
            message: message.clone(),
        },
        AzureBatchesError::Sdk { source } => Raised::Sdk(source.to_string()),
        AzureBatchesError::Invalid { .. } => Raised::Invalid(error.to_string()),
        AzureBatchesError::Endpoint { source } => Raised::Endpoint(source.to_string()),
    }
}

fn answered(reply: Reply) -> Answered {
    match reply {
        Reply::Batch(object) => Answered::Batch(object),
        Reply::Page(page) => Answered::Page(page),
    }
}

fn lock<T>(log: &Mutex<Vec<T>>) -> Vec<T>
where
    T: Clone,
{
    log.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

pub(super) fn observe(step: &Step) -> Observation {
    let calls = Calls::default();
    let seen = Shared::default();
    let materializer = Materializer {
        mode: step.mode,
        answer: step.answer.clone(),
        calls: Arc::clone(&calls),
        seen: Arc::clone(&seen),
    };
    let result = with_connection(step, &materializer, call_through);
    let raised = result.as_ref().err().map(raised);
    Observation {
        reply: result.ok().map(answered),
        raised,
        calls: lock(&calls),
        seen: lock(&seen),
    }
}

pub(super) fn project(step: &Step) -> Result<Projection, Raised> {
    let materializer = Materializer {
        mode: step.mode,
        answer: step.answer.clone(),
        calls: Calls::default(),
        seen: Shared::default(),
    };
    with_connection(step, &materializer, project_through)
}

pub(super) fn batch_object(body: &str) -> BatchObject {
    serde_json::from_str(body).expect("the expected batch parses")
}

pub(super) fn page_object(body: &str) -> BatchPage {
    serde_json::from_str(body).expect("the expected page parses")
}

pub(super) fn create_request(data: &str) -> BatchCreate {
    serde_json::from_str(data).expect("the expected create request parses")
}

pub(super) fn query_request(data: &str) -> BatchQuery {
    serde_json::from_str(data).expect("the expected query parses")
}

pub(super) fn status_failure() -> Failure {
    Failure {
        status: 400,
        error_type: Some("invalid_request_error".to_owned()),
        param: Some("input_file_id".to_owned()),
        code: Some("invalid_value".to_owned()),
        message: "the input file is not ready".to_owned(),
    }
}

mod scenarios;
