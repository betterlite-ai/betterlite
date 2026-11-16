#![cfg(test)]

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::sections::azure_endpoints::Settings as EndpointSettings;
use betterlite_openai::fine_tuning::JobRequest;
use betterlite_openai::fine_tuning::client::{BuildError, ClientSettings, JobsApi, SdkError};
use betterlite_openai::fine_tuning::job::{FineTuneCreate, FineTuneJob};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use super::client::{ClientMaterializer, Connection};
use super::error::AzureFineTuneError;
use super::{create_fine_tuning_job, list_fine_tuning_jobs, request_fine_tuning_job};
use crate::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use crate::credentials::cache::RefreshProviderCache;
use crate::credentials::source::{CallableTokenProvider, CredentialSources, SourceFailure};
use crate::endpoints::get_client::ClientCache;
use crate::endpoints::projection::{
    ClientIdentity, ClientInputs, ClientKind, ClientOutcome, SessionKind, V1Key,
};
use crate::oidc_exchange::OidcExchangeInput;

const ORG: &str = r#""org""#;
const MISSING: &str = "Azure OpenAI client is not initialized. Make sure api_key is passed or AZURE_API_KEY is set in the environment.";
const NOT_ASYNC: &str =
    "OpenAI client is not an instance of AsyncOpenAI. Make sure you passed an AsyncOpenAI client.";
const DEBUG_DEFAULT: &str = "Azure fine-tuning: defaulting trainingType=1 (supervised)";
const DEBUG_CREATE: &str = "creating fine tuning job, args=";
const DEBUG_LIST: &str = "list fine tuning job, after=";

type Calls = Arc<Mutex<Vec<Call>>>;
type Reply = Result<Vec<u8>, SdkError>;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    Create {
        body: String,
    },
    List {
        after: Option<String>,
        limit: Option<i64>,
    },
    Retrieve {
        job_id: String,
    },
    Cancel {
        job_id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Supplied {
    OpenAiSync,
    OpenAiAsync,
    AzureSync,
    AzureAsync,
}

impl Supplied {
    const fn is_async(self) -> bool {
        matches!(self, Self::OpenAiAsync | Self::AzureAsync)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum BuilderMode {
    #[default]
    Default,
    None,
    WrongAsync,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Project,
    Create,
    List,
    Retrieve,
    Cancel,
}

#[derive(Debug, Clone)]
struct Planned {
    status: u16,
    body: String,
}

#[derive(Debug, Clone)]
struct Step {
    action: Action,
    is_async: bool,
    client: Option<Supplied>,
    builder: BuilderMode,
    reply: Option<Planned>,
    data: Option<FineTuneCreate>,
    job_id: Option<String>,
    after: Option<String>,
    limit: Option<i64>,
    args: ClientSettings,
}

#[derive(Debug, Clone, PartialEq)]
struct Projection {
    class: &'static str,
    api_key: Option<String>,
    max_retries: Option<i64>,
    timeout: Option<f64>,
    organization: Option<String>,
    base_url: Option<String>,
    api_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Raised {
    class: &'static str,
    message: Option<String>,
    status: Option<u16>,
    error_type: Option<String>,
    param: Option<String>,
    code: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
enum Outcome {
    Job(String),
    Page(String),
    Projection(Projection),
    Raised(Raised),
}

#[derive(Debug, Clone, PartialEq)]
struct Observation {
    outcome: Outcome,
    calls: Vec<Call>,
    supplied_calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    error: ErrorFields,
}

#[derive(Debug, Deserialize)]
struct ErrorFields {
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    r#type: Option<String>,
    #[serde(default)]
    param: Option<String>,
    #[serde(default)]
    code: Option<String>,
}

struct Fake {
    is_async: bool,
    reply: Reply,
    calls: Calls,
}

impl Fake {
    fn answer(&self, call: Call) -> Reply {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
        self.reply.clone()
    }
}

impl JobsApi for Fake {
    fn is_async(&self) -> bool {
        self.is_async
    }

    fn create(&self, data: &FineTuneCreate) -> Reply {
        self.answer(Call::Create {
            body: wire_body(data),
        })
    }

    fn list(&self, after: Option<&str>, limit: Option<i64>) -> Reply {
        self.answer(Call::List {
            after: after.map(str::to_owned),
            limit,
        })
    }

    fn retrieve(&self, job_id: &str) -> Reply {
        self.answer(Call::Retrieve {
            job_id: job_id.to_owned(),
        })
    }

    fn cancel(&self, job_id: &str) -> Reply {
        self.answer(Call::Cancel {
            job_id: job_id.to_owned(),
        })
    }
}

struct Materializer {
    mode: BuilderMode,
    reply: Reply,
    calls: Calls,
}

impl ClientMaterializer for Materializer {
    fn materialize(
        &self,
        outcome: &ClientOutcome<'_>,
    ) -> Result<Option<Box<dyn JobsApi>>, BuildError> {
        if self.mode == BuilderMode::None {
            return Ok(None);
        }
        let constructed = self.mode == BuilderMode::Default
            && matches!(
                outcome.kind,
                ClientKind::AsyncAzureOpenAi | ClientKind::AsyncOpenAi
            );
        Ok(Some(Box::new(Fake {
            is_async: constructed,
            reply: self.reply.clone(),
            calls: Arc::clone(&self.calls),
        })))
    }
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
    fn get(&self, _identity: &ClientIdentity) -> Option<ClientKind> {
        None
    }

    fn set(&mut self, _identity: ClientIdentity, _kind: ClientKind, _owned: bool) {}
}

fn settings() -> ClientSettings {
    ClientSettings {
        api_key: Some(SecretString::from("key-1")),
        api_base: Some("https://fixture.test".to_owned()),
        api_version: Some("2024-10-21".to_owned()),
        timeout: None,
        max_retries: None,
        organization: None,
    }
}

fn ignored_settings() -> ClientSettings {
    let mut settings = settings();
    settings.timeout = Some(7.5_f64);
    settings.max_retries = Some(7);
    settings.organization = Some("org-x".to_owned());
    settings
}

fn step(action: Action) -> Step {
    Step {
        action,
        is_async: false,
        client: None,
        builder: BuilderMode::Default,
        reply: None,
        data: None,
        job_id: None,
        after: None,
        limit: None,
        args: settings(),
    }
}

fn reply(body: &str) -> Planned {
    Planned {
        status: 200,
        body: body.to_owned(),
    }
}

fn failure(status: u16, body: &str) -> Planned {
    Planned {
        status,
        body: body.to_owned(),
    }
}

fn create(data: &str, body: &str) -> Step {
    Step {
        data: Some(create_data(data)),
        reply: Some(reply(body)),
        ..step(Action::Create)
    }
}

fn retrieve(job_id: &str, body: &str) -> Step {
    Step {
        job_id: Some(job_id.to_owned()),
        reply: Some(reply(body)),
        ..step(Action::Retrieve)
    }
}

fn cancel(job_id: &str, body: &str) -> Step {
    Step {
        job_id: Some(job_id.to_owned()),
        reply: Some(reply(body)),
        ..step(Action::Cancel)
    }
}

fn list(body: &str) -> Step {
    Step {
        reply: Some(reply(body)),
        ..step(Action::List)
    }
}

fn project(client: Supplied, is_async: bool) -> Step {
    Step {
        is_async,
        client: Some(client),
        args: ignored_settings(),
        ..step(Action::Project)
    }
}

fn async_(step: Step) -> Step {
    Step {
        is_async: true,
        ..step
    }
}

fn supplied(step: Step, client: Supplied) -> Step {
    Step {
        client: Some(client),
        ..step
    }
}

fn built_by(step: Step, builder: BuilderMode) -> Step {
    Step { builder, ..step }
}

fn versioned(mut step: Step, version: &str) -> Step {
    step.args.api_version = Some(version.to_owned());
    step
}

fn after(step: Step, after: &str, limit: i64) -> Step {
    Step {
        after: Some(after.to_owned()),
        limit: Some(limit),
        ..step
    }
}

fn create_data(text: &str) -> FineTuneCreate {
    serde_json::from_str(text).expect("the create data parses")
}

fn wire_body(data: &FineTuneCreate) -> String {
    let mut body = serde_json::to_value(data).expect("the create data serializes");
    if let Some(map) = body.as_object_mut() {
        let merged = map
            .remove("extra_body")
            .and_then(|value| value.as_object().cloned());
        if let Some(fields) = merged {
            map.extend(fields);
        }
    }
    body.to_string()
}

fn job_json(status: &str, organization: &str, result_files: &str) -> String {
    format!(
        r#"{{"id":"ft_1","created_at":1,"error":null,"fine_tuned_model":null,"finished_at":null,"estimated_finish":null,"integrations":null,"metadata":null,"hyperparameters":{{"batch_size":null,"learning_rate_multiplier":null,"n_epochs":3}},"method":null,"model":"gpt-4o-mini","object":"fine_tuning.job","organization_id":{organization},"result_files":{result_files},"seed":5,"status":"{status}","trained_tokens":null,"training_file":"file_a","validation_file":null}}"#
    )
}

fn page(item: &str, has_more: bool) -> String {
    format!(r#"{{"object":"list","data":[{item}],"has_more":{has_more}}}"#)
}

fn canonical_job(text: &str) -> String {
    let job: FineTuneJob = serde_json::from_str(text).expect("the expected job parses");
    serde_json::to_string(&job).expect("the expected job serializes")
}

fn canonical_create(text: &str) -> Call {
    Call::Create {
        body: wire_body(&create_data(text)),
    }
}

fn error_fields(body: &str) -> ErrorFields {
    serde_json::from_str::<ErrorBody>(body)
        .expect("the error body parses")
        .error
}

fn planned_reply(planned: Option<&Planned>) -> Reply {
    let Some(Planned { status, body }) = planned else {
        return Ok(b"{}".to_vec());
    };
    if *status < 400 {
        return Ok(body.clone().into_bytes());
    }
    let fields = error_fields(body);
    Err(SdkError {
        status: *status,
        error_type: fields.r#type,
        param: fields.param,
        code: fields.code,
        message: fields.message.unwrap_or_default(),
    })
}

fn raised(error: &AzureFineTuneError) -> Raised {
    let mut report = Raised {
        class: "ValueError",
        message: Some(error.to_string()),
        status: None,
        error_type: None,
        param: None,
        code: None,
    };
    match error {
        AzureFineTuneError::Sdk { source } => {
            report.class = "APIStatusError";
            report.message = Some(source.message.clone());
            report.status = Some(source.status);
            report.error_type.clone_from(&source.error_type);
            report.param.clone_from(&source.param);
            report.code.clone_from(&source.code);
        }
        AzureFineTuneError::Build { source } => {
            report.class = "TypeError";
            report.message = Some(source.to_string());
        }
        AzureFineTuneError::Decode { .. } => {
            report.class = "ValidationError";
            report.message = None;
        }
        AzureFineTuneError::Endpoint { .. }
        | AzureFineTuneError::ClientMissing
        | AzureFineTuneError::NotAsyncClient => {}
    }
    report
}

fn v1_key(key: Option<&V1Key<'_>>) -> Option<String> {
    match key {
        Some(V1Key::Static(secret)) => Some(secret.expose_secret().to_owned()),
        Some(V1Key::Provider { .. }) | None => None,
    }
}

fn projection(outcome: &ClientOutcome<'_>) -> Projection {
    let class = match outcome.kind {
        ClientKind::AzureOpenAi => "AzureOpenAI",
        ClientKind::AsyncAzureOpenAi => "AsyncAzureOpenAI",
        ClientKind::OpenAi => "OpenAI",
        ClientKind::AsyncOpenAi => "AsyncOpenAI",
        ClientKind::Other => "Other",
    };
    let (api_key, max_retries, timeout) = match outcome.inputs.as_ref() {
        Some(ClientInputs::Azure(azure)) => (
            azure
                .api_key
                .as_ref()
                .map(|key| key.expose_secret().to_owned()),
            Some(azure.max_retries),
            azure.timeout,
        ),
        Some(ClientInputs::OpenAiV1(v1)) => {
            (v1_key(v1.key.as_ref()), Some(v1.max_retries), v1.timeout)
        }
        None => (None, None, None),
    };
    let (base_url, api_version) = rebuilt(outcome.inputs.as_ref());
    Projection {
        class,
        api_key,
        max_retries,
        timeout,
        organization: None,
        base_url,
        api_version,
    }
}

// the SDK turns azure_endpoint into <endpoint>/openai and keeps a deployment base_url, always ending in a slash
fn rebuilt(inputs: Option<&ClientInputs<'_>>) -> (Option<String>, Option<String>) {
    match inputs {
        Some(ClientInputs::Azure(azure)) => {
            let base_url = match (&azure.endpoint.azure_endpoint, &azure.endpoint.base_url) {
                (Some(endpoint), _) => Some(format!("{}/openai/", endpoint.trim_end_matches('/'))),
                (None, Some(path)) => Some(format!("{}/", path.trim_end_matches('/'))),
                (None, None) => None,
            };
            (base_url, Some(azure.api_version.clone()))
        }
        Some(ClientInputs::OpenAiV1(v1)) => (v1.base_url.clone(), None),
        None => (None, None),
    }
}

fn dispatch(
    step: &Step,
    data: &mut FineTuneCreate,
    mut connection: Connection<'_, '_, Materializer>,
) -> Result<Outcome, AzureFineTuneError> {
    let job_id = step.job_id.as_deref().unwrap_or_default();
    match step.action {
        Action::Project => connection
            .get_openai_client()
            .map(|outcome| Outcome::Projection(projection(&outcome))),
        Action::Create => create_fine_tuning_job(connection, data)
            .map(|job| Outcome::Job(serde_json::to_string(&job).expect("the job serializes"))),
        Action::Retrieve => request_fine_tuning_job(connection, &JobRequest::Retrieve(job_id))
            .map(|job| Outcome::Job(serde_json::to_string(&job).expect("the job serializes"))),
        Action::Cancel => request_fine_tuning_job(connection, &JobRequest::Cancel(job_id))
            .map(|job| Outcome::Job(serde_json::to_string(&job).expect("the job serializes"))),
        Action::List => list_fine_tuning_jobs(connection, step.after.as_deref(), step.limit)
            .map(|page| Outcome::Page(String::from_utf8(page).expect("the list reply is UTF-8"))),
    }
}

fn run(step: &Step) -> Observation {
    let calls = Calls::default();
    let supplied_calls = Calls::default();
    let reply = planned_reply(step.reply.as_ref());
    let (client, materializer) = fixtures(step, &reply, &calls, &supplied_calls);
    let mut data = step.data.clone().unwrap_or_default();
    Observation {
        outcome: connect(step, &mut data, client, &materializer),
        calls: lock(&calls),
        supplied_calls: lock(&supplied_calls),
    }
}

fn fixtures(
    step: &Step,
    reply: &Reply,
    calls: &Calls,
    supplied_calls: &Calls,
) -> (Option<Box<dyn JobsApi>>, Materializer) {
    let client = step.client.map(|kind| -> Box<dyn JobsApi> {
        Box::new(Fake {
            is_async: kind.is_async(),
            reply: reply.clone(),
            calls: Arc::clone(supplied_calls),
        })
    });
    let materializer = Materializer {
        mode: step.builder,
        reply: reply.clone(),
        calls: Arc::clone(calls),
    };
    (client, materializer)
}

fn connect(
    step: &Step,
    data: &mut FineTuneCreate,
    client: Option<Box<dyn JobsApi>>,
    materializer: &Materializer,
) -> Outcome {
    let endpoints = EndpointSettings::default();
    let environment: BTreeMap<String, String> = BTreeMap::new();
    let sources = Sources;
    let mut cache = Cache;
    let mut refresh = RefreshProviderCache::new(&sources);
    let connection = Connection {
        settings: step.args.clone(),
        client,
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
    match dispatch(step, data, connection) {
        Ok(outcome) => outcome,
        Err(error) => Outcome::Raised(raised(&error)),
    }
}

fn lock(calls: &Calls) -> Vec<Call> {
    calls.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

fn job_observation(text: &str, calls: Vec<Call>) -> Observation {
    Observation {
        outcome: Outcome::Job(canonical_job(text)),
        calls,
        supplied_calls: Vec::new(),
    }
}

#[derive(Clone, Default)]
struct Messages(Arc<Mutex<Vec<String>>>);

struct MessagesVisitor(String);

impl Visit for MessagesVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}").trim_matches('"').to_owned();
        }
    }
}

impl<S: tracing::Subscriber> Layer<S> for Messages {
    fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, S>) {
        let mut message = MessagesVisitor(String::new());
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.0);
    }
}

fn capture<T, F>(run: F) -> (T, Vec<String>)
where
    F: FnOnce() -> T,
{
    let messages = Messages::default();
    let subscriber = tracing_subscriber::registry().with(messages.clone());
    let result = with_default(subscriber, run);
    let captured = messages
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    (result, captured)
}

mod cases;
