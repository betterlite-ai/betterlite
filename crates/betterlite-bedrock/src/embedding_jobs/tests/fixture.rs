use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_http::proxy::transport_error::TransportError;
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
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use secrecy::SecretString;
use serde_json::value::RawValue;
use tokio::runtime::{Builder, Runtime};

use super::super::error::EmbeddingJobsError;
use super::super::logging::{StatusEvent, log_call};
use super::super::status::{AsyncInvokeStatusCall, StatusServices, get_async_invoke_status};
use super::super::transport::{StatusReply, StatusTransport, StatusWireRequest};
use crate::aws_settings::sources::Credentials;
use crate::bedrock_embedding_io::credentials::ManagedRegionWorker;
use crate::bedrock_embedding_io::transport::{
    ClientFactory, EmbeddingReply, EmbeddingRequest, EmbeddingTransport, Timeout, TransferError,
};
use crate::credentials::CredentialSource;
use crate::credentials::error::CredentialError;
use crate::image_call::plan::LogContext;
use crate::signing::executor::AwsSigningExecutor;

pub(super) const ACCESS_KEY: &str = "AKIDEXAMPLE";
pub(super) const SECRET_KEY: &str = "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY";
pub(super) const SESSION_TOKEN: &str = "session-token";
pub(super) const INVOCATION_ARN: &str = "arn:aws:bedrock:us-west-2:123456789012:async-invoke/job-1";
pub(super) const ENCODED_ARN: &str =
    "arn%3Aaws%3Abedrock%3Aus-west-2%3A123456789012%3Aasync-invoke%2Fjob-1";

// 1_700_000_000 seconds is 2023-11-14T22:13:20Z, the instant the signing assertions name
pub(super) fn now() -> SystemTime {
    SystemTime::UNIX_EPOCH
        .checked_add(Duration::from_secs(1_700_000_000))
        .expect("the fixture clock is in range")
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

pub(super) fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[derive(Clone, Debug)]
pub(super) enum Reply {
    Answer { status: u16, body: Vec<u8> },
    Failed,
    Timeout,
}

impl Reply {
    pub(super) fn ok(body: &str) -> Self {
        Self::status(200, body)
    }

    pub(super) fn status(status: u16, body: &str) -> Self {
        Self::bytes(status, body.as_bytes())
    }

    pub(super) fn bytes(status: u16, body: &[u8]) -> Self {
        Self::Answer {
            status,
            body: body.to_vec(),
        }
    }
}

#[derive(Clone)]
pub(super) struct Scenario {
    pub(super) endpoint: Option<String>,
    pub(super) optional_params: BTreeMap<String, Box<RawValue>>,
    pub(super) environment: BTreeMap<String, String>,
    pub(super) region: String,
    pub(super) credentials: Credentials,
    pub(super) reply: Reply,
    pub(super) logging: bool,
}

impl Scenario {
    pub(super) fn new(reply: Reply) -> Self {
        Self {
            endpoint: None,
            optional_params: BTreeMap::new(),
            environment: environment(&[("AWS_REGION", "eu-west-1")]),
            region: "us-west-2".to_owned(),
            credentials: credentials(),
            reply,
            logging: false,
        }
    }
}

pub(super) struct Source {
    pub(super) credentials: Credentials,
    pub(super) calls: Mutex<Vec<(AwsSettings, String)>>,
}

impl Source {
    pub(super) fn new(credentials: Credentials) -> Self {
        Self {
            credentials,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<(AwsSettings, String)> {
        self.calls.lock().unwrap().clone()
    }
}

impl CredentialSource for Source {
    fn resolve(
        &self,
        settings: AwsSettings,
        region: String,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send {
        self.calls.lock().unwrap().push((settings, region));
        ready(Ok(self.credentials.clone()))
    }
}

pub(super) struct FixtureClient {
    reply: Reply,
    seen: Arc<Mutex<Vec<StatusWireRequest>>>,
}

impl StatusTransport for FixtureClient {
    fn get(
        &self,
        request: StatusWireRequest,
    ) -> impl Future<Output = Result<StatusReply, TransferError>> + Send {
        self.seen.lock().unwrap().push(request);
        let answer = match &self.reply {
            Reply::Answer { status, body } => Ok(StatusReply {
                status: *status,
                body: body.clone(),
            }),
            Reply::Failed => Err(TransferError::Failed(TransportError::Closed)),
            Reply::Timeout => Err(TransferError::Timeout {
                elapsed: Duration::from_secs(7),
            }),
        };
        ready(answer)
    }
}

// B173's ClientFactory needs a post for the embedding calls this task does not make
impl EmbeddingTransport for FixtureClient {
    fn post(
        &self,
        _request: EmbeddingRequest,
    ) -> impl Future<Output = Result<EmbeddingReply, TransferError>> + Send {
        ready(Err(TransferError::Failed(TransportError::Closed)))
    }
}

pub(super) struct Factory {
    reply: Reply,
    seen: Arc<Mutex<Vec<StatusWireRequest>>>,
    calls: Mutex<Vec<(String, Option<Timeout>)>>,
}

impl Factory {
    pub(super) fn new(reply: Reply) -> Self {
        Self {
            reply,
            seen: Arc::new(Mutex::new(Vec::new())),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn requests(&self) -> Vec<StatusWireRequest> {
        self.seen.lock().unwrap().clone()
    }

    pub(super) fn calls(&self) -> Vec<(String, Option<Timeout>)> {
        self.calls.lock().unwrap().clone()
    }
}

impl ClientFactory for Factory {
    type Client = FixtureClient;

    fn sync_client(&self, _timeout: Option<Timeout>) -> Self::Client {
        FixtureClient {
            reply: self.reply.clone(),
            seen: Arc::clone(&self.seen),
        }
    }

    fn async_client(&self, provider: &str, timeout: Option<Timeout>) -> Self::Client {
        self.calls
            .lock()
            .unwrap()
            .push((provider.to_owned(), timeout));
        FixtureClient {
            reply: self.reply.clone(),
            seen: Arc::clone(&self.seen),
        }
    }
}

struct NoPorts;

impl OidcHttp for NoPorts {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: "the fixture has no OIDC transport".to_owned(),
        })
    }
}

impl AzureOidcTokens for NoPorts {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Ok(None)
    }
}

impl OidcClock for NoPorts {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn unix_seconds(&self) -> i64 {
        0
    }
}

pub(super) fn secret_lookup() -> SecretLookup {
    SecretLookup {
        manager: None,
        oidc_http: Arc::new(NoPorts),
        azure_tokens: Arc::new(NoPorts),
        clock: Arc::new(NoPorts),
        oidc_cache: OidcTokenCache::default(),
    }
}

#[derive(Default)]
pub(super) struct DiscardingSink;

impl LogSink for DiscardingSink {
    fn emit(&self, _record: LogRecord) {}
}

pub(super) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the default redactor builds")
}

pub(super) fn logging_state() -> LoggingState {
    let init: LoggingInit = serde_json::from_str(
        r#"{"model":"embedding-model","call_type":"embeddings","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
    )
    .expect("the logging fixture is valid");
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        &redactor(),
        || "generated".to_owned(),
        Duration::default,
    )
    .expect("the logging fixture builds")
}

pub(super) struct Run {
    pub(super) result: Result<Json, EmbeddingJobsError>,
    pub(super) requests: Vec<StatusWireRequest>,
    pub(super) factory_calls: Vec<(String, Option<Timeout>)>,
    pub(super) resolved: Vec<(AwsSettings, String)>,
    pub(super) snapshots: Vec<Json>,
}

fn parameters(scenario: &Scenario) -> BTreeMap<String, Box<RawValue>> {
    let mut parameters = scenario.optional_params.clone();
    if let Some(endpoint) = &scenario.endpoint {
        parameters.insert(
            "aws_bedrock_runtime_endpoint".to_owned(),
            RawValue::from_string(serde_json::to_string(endpoint).unwrap())
                .expect("the endpoint serializes"),
        );
    }
    parameters
}

pub(super) fn prepost_env<'env>(
    state: &LoggingState,
    redactor: &'env Redactor,
    sink: &'env DiscardingSink,
    logger: &'env (dyn Fn(&Json) -> Result<(), PrePostError> + Sync),
) -> PrePostEnv<'env> {
    PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: sink,
        input_callbacks: &[],
        sentry: None,
        logger_fn: Some(logger),
        now: state.start_time,
    }
}

fn snapshot_logger(snapshots: Arc<Mutex<Vec<Json>>>) -> impl Fn(&Json) -> Result<(), PrePostError> {
    move |snapshot: &Json| -> Result<(), PrePostError> {
        snapshots.lock().unwrap().push(snapshot.clone());
        Ok(())
    }
}

pub(super) fn run(scenario: &Scenario) -> Run {
    let factory = Factory::new(scenario.reply.clone());
    let source = Source::new(scenario.credentials.clone());
    let secrets = secret_lookup();
    let region_worker = ManagedRegionWorker::new().expect("the region worker starts");
    let snapshots: Arc<Mutex<Vec<Json>>> = Arc::new(Mutex::new(Vec::new()));
    let signing = AwsSigningExecutor::default();
    let services = StatusServices {
        source: &source,
        secrets: &secrets,
        region_worker: &region_worker,
        factory: &factory,
        signing: &signing,
    };
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the fixture runtime builds");
    let result = execute_status(scenario, &services, &runtime, Arc::clone(&snapshots));
    Run {
        result,
        requests: factory.requests(),
        factory_calls: factory.calls(),
        resolved: source.calls(),
        snapshots: snapshots.lock().unwrap().clone(),
    }
}

fn execute_status(
    scenario: &Scenario,
    services: &StatusServices<'_, Source, Factory>,
    runtime: &Runtime,
    snapshots: Arc<Mutex<Vec<Json>>>,
) -> Result<Json, EmbeddingJobsError> {
    let mut optional_params = parameters(scenario);
    let mut state = logging_state();
    let logger = snapshot_logger(snapshots);
    let redactor = redactor();
    let sink = DiscardingSink;
    let mut logger_callback = |arn: &str, event: StatusEvent<'_>| {
        let env = prepost_env(&state, &redactor, &sink, &logger);
        log_call(
            arn,
            event,
            &mut LogContext {
                state: &mut state,
                env: &env,
            },
        )
    };
    let call = AsyncInvokeStatusCall {
        invocation_arn: INVOCATION_ARN,
        region: &scenario.region,
        optional_params: &mut optional_params,
        environment: &scenario.environment,
        now: now(),
    };
    runtime.block_on(get_async_invoke_status(
        call,
        services,
        scenario.logging.then_some(&mut logger_callback),
    ))
}
