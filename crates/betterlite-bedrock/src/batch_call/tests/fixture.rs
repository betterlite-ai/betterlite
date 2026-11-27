use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_openai::batches::records::BatchObject;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::value::RawValue;
use tokio::runtime::{Builder, Runtime};

use crate::aws_settings::sources::Credentials;
use crate::batch_call::cancel::{CancelBatchCall, cancel_batch};
use crate::batch_call::error::BatchCallError;
use crate::batch_call::logging::{CallEvent, CallLogger, log_call};
use crate::batch_call::status::{JobStatusCall, retrieve_model_job_status};
use crate::batch_call::transport::{
    ClientConfig, ControlPlaneFactory, ControlPlaneReply, ControlPlaneRequest,
    ControlPlaneTransport,
};
use crate::bedrock_embedding_io::transport::TransferError;
use crate::credentials::error::CredentialError;
use crate::image_call::plan::{CredentialSource, LogContext};

pub(super) const ACCESS_KEY: &str = "AKIDEXAMPLE";
pub(super) const SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY";
pub(super) const SESSION_TOKEN: &str = "session-token";
pub(super) const CONFIGURED_ACCESS_KEY: &str = "AKIDCONFIGURED";
pub(super) const CONFIGURED_SECRET_KEY: &str = "configured-secret-key";
pub(super) const CONFIGURED_SESSION_TOKEN: &str = "configured-session-token";
pub(super) const REGION: &str = "eu-west-1";
pub(super) const JOB_ID: &str = "job-1";
pub(super) const JOB_ARN: &str =
    "arn:aws:bedrock:eu-west-1:123456789012:model-invocation-job/job-1";
pub(super) const ENCODED_JOB_ARN: &str =
    "arn%3Aaws%3Abedrock%3Aeu-west-1%3A123456789012%3Amodel-invocation-job%2Fjob-1";
pub(super) const SUBMITTED_EPOCH: i64 = 1_705_314_600;

pub(super) fn now() -> SystemTime {
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(1_700_000_000))
        .expect("the fixture clock is in range")
}

pub(super) fn runtime() -> Runtime {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the fixture runtime builds")
}

pub(super) fn credentials() -> Credentials {
    credentials_with(ACCESS_KEY, SECRET_KEY, Some(SESSION_TOKEN))
}

pub(super) fn credentials_with(
    access_key_id: &str,
    secret_access_key: &str,
    session_token: Option<&str>,
) -> Credentials {
    Credentials {
        access_key_id: SecretString::from(access_key_id.to_owned()),
        secret_access_key: SecretString::from(secret_access_key.to_owned()),
        session_token: session_token.map(|token| SecretString::from(token.to_owned())),
    }
}

pub(super) fn parameter(name: &str, value: Json) -> (String, Box<RawValue>) {
    let encoded = serde_json::to_string(&value).expect("the parameter encodes");
    let raw = RawValue::from_string(encoded).expect("the parameter is JSON");
    (name.to_owned(), raw)
}

pub(super) fn parameters(entries: &[(&str, Json)]) -> BTreeMap<String, Box<RawValue>> {
    entries
        .iter()
        .map(|(name, value)| parameter(name, value.clone()))
        .collect()
}

// the ten explicitly named credential arguments cancel_batch takes, plus one key it must ignore
pub(super) fn configured_auth() -> Vec<(&'static str, Json)> {
    let tags = Json::List(vec![Json::Object(indexmap::IndexMap::from([
        ("Key".to_owned(), Json::Text("team".to_owned())),
        ("Value".to_owned(), Json::Text("batches".to_owned())),
    ]))]);
    vec![
        (
            "aws_access_key_id",
            Json::Text(CONFIGURED_ACCESS_KEY.to_owned()),
        ),
        (
            "aws_secret_access_key",
            Json::Text(CONFIGURED_SECRET_KEY.to_owned()),
        ),
        (
            "aws_session_token",
            Json::Text(CONFIGURED_SESSION_TOKEN.to_owned()),
        ),
        ("aws_session_name", Json::Text("session".to_owned())),
        ("aws_profile_name", Json::Text("profile".to_owned())),
        ("aws_role_name", Json::Text("role".to_owned())),
        (
            "aws_web_identity_token",
            Json::Text("web-identity".to_owned()),
        ),
        (
            "aws_sts_endpoint",
            Json::Text("https://sts.example".to_owned()),
        ),
        ("aws_external_id", Json::Text("external".to_owned())),
        ("aws_session_tags", tags),
        ("some_unknown_key", Json::Text("ignored".to_owned())),
    ]
}

pub(super) fn auth_parameters() -> BTreeMap<String, Box<RawValue>> {
    parameters(&configured_auth())
}

struct SourceState {
    credentials: Credentials,
    fail: bool,
    settings: Vec<AwsSettings>,
    regions: Vec<String>,
}

pub(super) struct Source {
    state: Arc<Mutex<SourceState>>,
}

impl Source {
    pub(super) fn new(credentials: Credentials) -> Self {
        Self {
            state: Arc::new(Mutex::new(SourceState {
                credentials,
                fail: false,
                settings: Vec::new(),
                regions: Vec::new(),
            })),
        }
    }

    pub(super) fn failing() -> Self {
        let mut source = Self::new(credentials());
        source.state.lock().expect("the source lock").fail = true;
        source
    }

    pub(super) fn regions(&self) -> Vec<String> {
        self.state.lock().expect("the source lock").regions.clone()
    }

    pub(super) fn settings(&self) -> Vec<AwsSettings> {
        self.state.lock().expect("the source lock").settings.clone()
    }
}

impl CredentialSource for Source {
    fn resolve(
        &self,
        settings: AwsSettings,
        region: String,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send {
        let mut state = self.state.lock().expect("the source lock");
        state.settings.push(settings);
        state.regions.push(region);
        let outcome = if state.fail {
            Err(CredentialError::MissingCredentials)
        } else {
            Ok(state.credentials.clone())
        };
        drop(state);
        ready(outcome)
    }
}

#[derive(Clone, Debug)]
pub(super) struct ClientConfigRecord {
    pub(super) service_name: String,
    pub(super) region: String,
    pub(super) signature_version: String,
    pub(super) access_key_id: String,
    pub(super) secret_key_present: bool,
    pub(super) session_token_present: bool,
}

pub(super) enum Reply {
    Answer(ControlPlaneReply),
    Fail,
}

pub(super) fn answer(status: u16, body: &str) -> Reply {
    Reply::Answer(ControlPlaneReply {
        status,
        headers: Vec::new(),
        body: body.as_bytes().to_vec(),
    })
}

pub(super) fn answer_with(status: u16, headers: &[(&str, &str)], body: &str) -> Reply {
    let headers = headers
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    Reply::Answer(ControlPlaneReply {
        status,
        headers,
        body: body.as_bytes().to_vec(),
    })
}

#[derive(Default)]
struct FactoryState {
    configs: Vec<ClientConfigRecord>,
    requests: Vec<ControlPlaneRequest>,
    replies: VecDeque<Reply>,
}

pub(super) struct Factory {
    state: Arc<Mutex<FactoryState>>,
}

impl Factory {
    pub(super) fn new(replies: Vec<Reply>) -> Self {
        let mut state = FactoryState {
            configs: Vec::new(),
            requests: Vec::new(),
            replies: VecDeque::new(),
        };
        state.replies.extend(replies);
        Self {
            state: Arc::new(Mutex::new(state)),
        }
    }

    pub(super) fn configs(&self) -> Vec<ClientConfigRecord> {
        self.state.lock().expect("the factory lock").configs.clone()
    }

    pub(super) fn requests(&self) -> Vec<ControlPlaneRequest> {
        self.state
            .lock()
            .expect("the factory lock")
            .requests
            .clone()
    }
}

impl ControlPlaneFactory for Factory {
    type Client = Client;

    fn client(&self, config: ClientConfig<'_>) -> Client {
        let record = ClientConfigRecord {
            service_name: config.service_name.to_owned(),
            region: config.region.to_owned(),
            signature_version: config.signature_version.to_owned(),
            access_key_id: config.credentials.access_key_id.expose_secret().to_owned(),
            secret_key_present: !config
                .credentials
                .secret_access_key
                .expose_secret()
                .is_empty(),
            session_token_present: config.credentials.session_token.is_some(),
        };
        self.state
            .lock()
            .expect("the factory lock")
            .configs
            .push(record);
        Client {
            state: Arc::clone(&self.state),
        }
    }
}

struct Client {
    state: Arc<Mutex<FactoryState>>,
}

impl ControlPlaneTransport for Client {
    fn send(
        &self,
        request: ControlPlaneRequest,
    ) -> impl Future<Output = Result<ControlPlaneReply, TransferError>> + Send {
        let queued = {
            let mut state = self.state.lock().expect("the factory lock");
            state.requests.push(request);
            state.replies.pop_front()
        };
        let outcome = match queued {
            Some(Reply::Answer(reply)) => Ok(reply),
            Some(Reply::Fail) | None => Err(TransferError::Timeout {
                elapsed: Duration::ZERO,
            }),
        };
        ready(outcome)
    }
}

#[derive(Default)]
struct DiscardingSink;

impl LogSink for DiscardingSink {
    fn emit(&self, _record: LogRecord) {}
}

pub(super) fn logged<Output, Invoke>(invoke: Invoke) -> (Output, Vec<Json>)
where
    Invoke: FnOnce(Option<&mut CallLogger<'_>>) -> Output,
{
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the fixture redactor builds");
    let mut state = logging_state(&redactor);
    let snapshots: Mutex<Vec<Json>> = Mutex::new(Vec::new());
    let mut callback = |batch_id: &str, event: CallEvent<'_>| {
        capture_event(&mut state, &redactor, &snapshots, batch_id, event)
    };
    let result = invoke(Some(&mut callback));
    let captured = snapshots.into_inner().expect("the snapshot lock");
    (result, captured)
}

fn capture_event(
    state: &mut LoggingState,
    redactor: &Redactor,
    snapshots: &Mutex<Vec<Json>>,
    batch_id: &str,
    event: CallEvent<'_>,
) -> Result<(), PrePostError> {
    let logger = |snapshot: &Json| -> Result<(), PrePostError> {
        snapshots
            .lock()
            .expect("the snapshot lock")
            .push(snapshot.clone());
        Ok(())
    };
    let env = PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: &DiscardingSink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: Some(&logger),
        now: state.start_time,
    };
    log_call(batch_id, event, &mut LogContext { state, env: &env })
}

fn logging_state(redactor: &Redactor) -> LoggingState {
    let init: LoggingInit = serde_json::from_str(
        r#"{"model":"batch-model","call_type":"batches","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
    )
    .expect("the logging fixture is valid");
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || "generated".to_owned(),
        Duration::default,
    )
    .expect("the logging fixture builds")
}

pub(super) fn field<'json>(value: &'json Json, name: &str) -> &'json Json {
    match value {
        Json::Object(fields) => fields.get(name).unwrap_or(&Json::Null),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => &Json::Null,
    }
}

pub(super) fn text(value: &Json) -> &str {
    match value {
        Json::Text(value) => value,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            panic!("the fixture value is text")
        }
    }
}

pub(super) struct CallRun {
    pub(super) result: Result<BatchObject, BatchCallError>,
    pub(super) factory: Factory,
    pub(super) source: Source,
}

// GetModelInvocationJob through the same seam the handler uses, with one canned reply
pub(super) fn lookup_job(
    body: &str,
    batch_id: &str,
    region: Option<&str>,
    kwargs: &BTreeMap<String, Box<RawValue>>,
) -> CallRun {
    run_job(answer(200, body), batch_id, region, kwargs)
}

pub(super) enum JobCall {
    Lookup(Reply),
    Stop(Vec<Reply>),
}

impl From<Reply> for JobCall {
    fn from(reply: Reply) -> Self {
        Self::Lookup(reply)
    }
}

impl From<Vec<Reply>> for JobCall {
    fn from(replies: Vec<Reply>) -> Self {
        Self::Stop(replies)
    }
}

// both control plane calls run through the seam the handler uses, with canned replies in order
pub(super) fn run_job<Call>(
    call: Call,
    batch_id: &str,
    region: Option<&str>,
    kwargs: &BTreeMap<String, Box<RawValue>>,
) -> CallRun
where
    JobCall: From<Call>,
{
    let source = Source::new(credentials());
    let (factory, result) = match JobCall::from(call) {
        JobCall::Lookup(reply) => {
            let factory = Factory::new(vec![reply]);
            let request = JobStatusCall {
                batch_id,
                aws_region_name: region,
                kwargs,
                now: now(),
            };
            let result =
                runtime().block_on(retrieve_model_job_status(request, &source, &factory, None));
            (factory, result)
        }
        JobCall::Stop(replies) => {
            let factory = Factory::new(replies);
            let request = CancelBatchCall {
                batch_id,
                aws_region_name: region,
                kwargs,
                now: now(),
            };
            let result = runtime().block_on(cancel_batch(request, &source, &factory, None));
            (factory, result)
        }
    };
    CallRun {
        result,
        factory,
        source,
    }
}
