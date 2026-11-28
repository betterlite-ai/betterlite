use std::collections::{BTreeMap, VecDeque};
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_core::api::vector_store_http_write::service::{ClientKind, SuppliedClient};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
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
use betterlite_openai::cohere_embedding::TextEncoding;
use betterlite_openai::cohere_legacy_embedding::execute::LegacyEmbeddingTransport;
use betterlite_prompts::arguments::json::Object;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::ports::{
    AzureOidcError, AzureOidcTokens, OidcClock, OidcHttp, OidcHttpError, OidcRequest, OidcResponse,
    OidcTokenCache,
};
use betterlite_tokenizer::engine::error::EngineError;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::value::RawValue;
use tokio::runtime::{Builder, Runtime};

use crate::aws_settings::sources::Credentials;
use crate::bedrock_embedding_io::credentials::ManagedRegionWorker;
use crate::bedrock_embedding_io::transport::{
    ClientFactory, EmbeddingReply, EmbeddingRequest, EmbeddingTransport, ReplyRequest, Timeout,
    TransferError,
};
use crate::credentials::error::CredentialError;
use crate::embedding_call::error::EmbeddingCallError;
use crate::embedding_call::plan::remaining_options;
use crate::embedding_call::{
    Clock, CredentialSource, Dependencies, EmbeddingCall, EmbeddingCallResponse,
    EmbeddingEnvironment, LogContext, ParamMap, embeddings,
};
use crate::signing::executor::AwsSigningExecutor;

// 1_700_000_000 seconds is 2023-11-14T22:13:20Z, the instant the signing assertions name
pub(super) fn now() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(1_700_000_000))
        .expect("the fixture clock is in range")
}

// a scripted clock hands out its times in order and then stays at the fallback
pub(super) struct SteppingClock {
    times: Mutex<VecDeque<SystemTime>>,
    fallback: SystemTime,
}

impl SteppingClock {
    pub(super) fn new(times: Vec<SystemTime>, fallback: SystemTime) -> Self {
        Self {
            times: Mutex::new(VecDeque::from(times)),
            fallback,
        }
    }
}

impl Clock for SteppingClock {
    fn now(&self) -> SystemTime {
        self.times
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(self.fallback)
    }
}

pub(super) fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

#[derive(Clone, Debug)]
pub(super) enum Reply {
    Answer {
        status: u16,
        headers: Vec<(String, String)>,
        body: String,
    },
    Timeout,
    Failed,
}

impl Reply {
    pub(super) fn ok(body: &str) -> Self {
        Self::status(200, &[], body)
    }

    pub(super) fn status(status: u16, headers: &[(&str, &str)], body: &str) -> Self {
        Self::Answer {
            status,
            headers: headers
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
            body: body.to_owned(),
        }
    }
}

impl Default for Reply {
    fn default() -> Self {
        Self::ok("{}")
    }
}

#[derive(Clone, Default)]
pub(super) struct Replies(Arc<Mutex<VecDeque<Reply>>>);

impl Replies {
    pub(super) fn queue(replies: &[Reply]) -> Self {
        Self(Arc::new(Mutex::new(replies.iter().cloned().collect())))
    }

    fn take(&self) -> Reply {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Recorded {
    pub(super) url: String,
    pub(super) headers: BTreeMap<String, String>,
    pub(super) body: Vec<u8>,
    pub(super) deadline: Option<Duration>,
}

#[derive(Clone, Default)]
pub(super) struct Seen(Arc<Mutex<Vec<Recorded>>>);

impl Seen {
    pub(super) fn requests(&self) -> Vec<Recorded> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn push(&self, request: Recorded) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
    }
}

// the fixture client stands in for both the HTTPHandler and the AsyncHTTPHandler and records what it sent
pub(super) struct FixtureClient {
    replies: Replies,
    seen: Seen,
}

impl FixtureClient {
    pub(super) fn new(replies: Replies) -> Self {
        Self {
            replies,
            seen: Seen::default(),
        }
    }

    pub(super) fn requests(&self) -> Vec<Recorded> {
        self.seen.requests()
    }

    fn answer(&self) -> Reply {
        self.replies.take()
    }

    fn reply(&self, url: &str) -> Result<EmbeddingReply, TransferError> {
        match self.answer() {
            Reply::Answer {
                status,
                headers,
                body,
            } => Ok(EmbeddingReply {
                status,
                headers,
                body: body.into_bytes(),
                request: ReplyRequest {
                    method: String::from("POST"),
                    url: url.to_owned(),
                },
            }),
            Reply::Timeout => Err(TransferError::Timeout {
                elapsed: Duration::from_millis(125),
            }),
            Reply::Failed => Err(TransferError::Failed(TransportError::Closed)),
        }
    }
}

impl EmbeddingTransport for FixtureClient {
    fn post(
        &self,
        request: EmbeddingRequest,
    ) -> impl Future<Output = Result<EmbeddingReply, TransferError>> + Send {
        self.seen.push(Recorded {
            url: request.url.clone(),
            headers: request.headers.into_iter().collect(),
            body: request.body,
            deadline: None,
        });
        ready(self.reply(&request.url))
    }
}

impl LegacyEmbeddingTransport for FixtureClient {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.seen.push(recorded(request));
        ready(upstream(self.answer()))
    }
}

fn upstream(answer: Reply) -> Result<UpstreamResponse, TransportError> {
    match answer {
        Reply::Answer {
            status,
            headers,
            body,
        } => Ok(UpstreamResponse {
            status,
            headers: headers
                .into_iter()
                .filter_map(|(name, value)| {
                    Some((
                        http::HeaderName::try_from(name).ok()?,
                        http::HeaderValue::try_from(value).ok()?,
                    ))
                })
                .collect(),
            body: body.into_bytes(),
        }),
        Reply::Timeout | Reply::Failed => Err(TransportError::Closed),
    }
}

fn recorded(request: OutboundRequest) -> Recorded {
    let payload = match request.body {
        RequestBody::Json(bytes) => bytes,
        RequestBody::Bytes { data, .. } => data,
        RequestBody::Empty | RequestBody::Multipart { .. } => Vec::new(),
    };
    Recorded {
        url: request.url.to_string(),
        headers: request
            .headers
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_owned(),
                    value.to_str().unwrap_or_default().to_owned(),
                )
            })
            .collect(),
        body: payload,
        deadline: Some(request.deadline),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ClientCall {
    pub(super) kind: ClientKind,
    pub(super) provider: Option<String>,
    pub(super) timeout: Option<Timeout>,
}

pub(super) struct Factory {
    replies: Replies,
    seen: Seen,
    calls: Mutex<Vec<ClientCall>>,
}

impl Factory {
    pub(super) fn new(replies: Replies) -> Self {
        Self {
            replies,
            seen: Seen::default(),
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn requests(&self) -> Vec<Recorded> {
        self.seen.requests()
    }

    pub(super) fn calls(&self) -> Vec<ClientCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn client(&self) -> FixtureClient {
        FixtureClient {
            replies: self.replies.clone(),
            seen: self.seen.clone(),
        }
    }

    fn add(&self, call: ClientCall) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }
}

impl ClientFactory for Factory {
    type Client = FixtureClient;

    fn sync_client(&self, timeout: Option<Timeout>) -> Self::Client {
        self.add(ClientCall {
            kind: ClientKind::Sync,
            provider: None,
            timeout,
        });
        self.client()
    }

    fn async_client(&self, provider: &str, timeout: Option<Timeout>) -> Self::Client {
        self.add(ClientCall {
            kind: ClientKind::Asynchronous,
            provider: Some(provider.to_owned()),
            timeout,
        });
        self.client()
    }
}

pub(super) struct Source {
    credentials: Credentials,
    calls: Mutex<Vec<String>>,
}

impl Source {
    pub(super) fn new(access: &str, secret_key: &str, token: Option<&str>) -> Self {
        Self {
            credentials: Credentials {
                access_key_id: secret(access),
                secret_access_key: secret(secret_key),
                session_token: token.map(secret),
            },
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn regions(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl CredentialSource for Source {
    fn resolve(
        &self,
        _settings: AwsSettings,
        region: String,
    ) -> impl Future<Output = Result<Credentials, CredentialError>> + Send {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(region);
        ready(Ok(self.credentials.clone()))
    }
}

struct NoOidc;

impl OidcHttp for NoOidc {
    fn get(&self, _request: OidcRequest) -> Result<OidcResponse, OidcHttpError> {
        Err(OidcHttpError::Request {
            message: String::from("the fixture has no OIDC transport"),
        })
    }
}

impl AzureOidcTokens for NoOidc {
    fn token(&self, _scope: &str) -> Result<Option<SecretString>, AzureOidcError> {
        Ok(None)
    }
}

impl OidcClock for NoOidc {
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
        oidc_http: Arc::new(NoOidc),
        azure_tokens: Arc::new(NoOidc),
        clock: Arc::new(NoOidc),
        oidc_cache: OidcTokenCache::default(),
    }
}

pub(super) struct Counting;

impl TextEncoding for Counting {
    fn count_tokens(&self, text: &str) -> Result<usize, EngineError> {
        Ok(text.chars().count())
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

pub(super) fn logging_state(redactor: &Redactor) -> LoggingState {
    let init: LoggingInit = serde_json::from_str(
        r#"{"model":"amazon.titan-embed-text-v1","call_type":"embedding","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
    )
    .expect("the logging fixture is valid");
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("generated"),
        Duration::default,
    )
    .expect("the logging fixture builds")
}

pub(super) type Logger<'env> = &'env (dyn Fn(&Json) -> Result<(), PrePostError> + Sync);

pub(super) fn prepost_env<'env>(
    state: &LoggingState,
    redactor: &'env Redactor,
    sink: &'env DiscardingSink,
    logger: Option<Logger<'env>>,
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
        logger_fn: logger,
        now: state.start_time,
    }
}

pub(super) fn runtime() -> Runtime {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the fixture runtime builds")
}

// options reach the call as raw JSON values, the way a parsed request body carries them
pub(super) fn raw_params(pairs: &[(&str, &str)]) -> ParamMap {
    pairs
        .iter()
        .map(|(name, value)| {
            (
                (*name).to_owned(),
                RawValue::from_string((*value).to_owned()).expect("the fixture value is raw JSON"),
            )
        })
        .collect()
}

pub(super) fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("the fixture JSON is an object")
}

// the raw request capture either logs the whole call or stops at the captured request
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum RawRequest {
    #[default]
    Full,
    Only,
}

#[derive(Clone)]
pub(super) struct Probe {
    pub(super) model: String,
    pub(super) input: Vec<String>,
    pub(super) optional_params: Vec<(&'static str, &'static str)>,
    pub(super) extra_headers: Option<Vec<(String, String)>>,
    pub(super) api_base: Option<String>,
    pub(super) api_key: Option<String>,
    pub(super) bearer: Option<String>,
    pub(super) environment: Vec<(&'static str, &'static str)>,
    pub(super) runtime_endpoint: Option<String>,
    pub(super) timeout: Option<Timeout>,
    pub(super) aembedding: bool,
    pub(super) drop_params: bool,
    pub(super) replies: Vec<Reply>,
    pub(super) supplied: Option<ClientKind>,
    pub(super) access_key: String,
    pub(super) secret_key: String,
    pub(super) session_token: Option<String>,
    pub(super) clock: Vec<SystemTime>,
    pub(super) raw_request: RawRequest,
}

pub(super) fn fixture_probe() -> Probe {
    Probe {
        model: String::from("amazon.titan-embed-text-v1"),
        input: vec![String::from("hello")],
        optional_params: vec![
            ("aws_access_key_id", "\"AKIAIOSFODNN7EXAMPLE\""),
            (
                "aws_secret_access_key",
                "\"wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY\"",
            ),
            ("aws_region_name", "\"us-east-1\""),
        ],
        extra_headers: None,
        api_base: None,
        api_key: None,
        bearer: None,
        environment: Vec::new(),
        runtime_endpoint: None,
        timeout: None,
        aembedding: false,
        drop_params: false,
        replies: Vec::new(),
        supplied: None,
        access_key: String::from("AKIAIOSFODNN7EXAMPLE"),
        secret_key: String::from("wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY"),
        session_token: Some(String::from("session-token")),
        clock: Vec::new(),
        raw_request: RawRequest::Full,
    }
}

pub(super) struct Run {
    pub(super) outcome: Result<EmbeddingCallResponse, EmbeddingCallError>,
    pub(super) factory_requests: Vec<Recorded>,
    pub(super) supplied_requests: Vec<Recorded>,
    pub(super) cohere_requests: Vec<Recorded>,
    pub(super) calls: Vec<ClientCall>,
    pub(super) regions: Vec<String>,
    pub(super) remaining: Object,
    pub(super) state: LoggingState,
    pub(super) snapshots: Vec<Json>,
}

impl Run {
    pub(super) fn error(&self) -> &EmbeddingCallError {
        self.outcome.as_ref().expect_err("the call fails")
    }

    pub(super) fn response(&self) -> &EmbeddingCallResponse {
        self.outcome.as_ref().expect("the call succeeds")
    }

    pub(super) fn posted(&self) -> &Recorded {
        let requests = if !self.supplied_requests.is_empty() {
            &self.supplied_requests
        } else if !self.factory_requests.is_empty() {
            &self.factory_requests
        } else {
            &self.cohere_requests
        };
        assert_eq!(requests.len(), 1, "one request reaches the provider");
        &requests[0]
    }

    pub(super) fn headers(&self) -> &BTreeMap<String, String> {
        &self.posted().headers
    }

    pub(super) fn body(&self) -> Object {
        serde_json::from_slice(&self.posted().body).expect("the sent body is a JSON object")
    }

    pub(super) fn pre_call_input(&self) -> Option<&Json> {
        self.state.model_call_details.get("input")
    }

    pub(super) fn original_response(&self) -> Option<&Json> {
        self.state.model_call_details.get("original_response")
    }

    pub(super) fn details(&self) -> &IndexMap<String, Json> {
        &self.state.model_call_details
    }
}

pub(super) fn run(probe: &Probe) -> Run {
    let source = Source::new(
        &probe.access_key,
        &probe.secret_key,
        probe.session_token.as_deref(),
    );
    run_with_source(probe, &source)
}

pub(super) fn run_with_source(probe: &Probe, source: &Source) -> Run {
    let environment = environment(probe);
    let api_key = probe.api_key.as_deref().map(secret);
    let bearer = probe.bearer.as_deref().map(secret);
    let mut call = probe_call(probe, &environment, api_key.as_ref(), bearer.as_ref());
    let fixtures = fixtures(probe, source);
    let redactor = redactor();
    let sink = DiscardingSink;
    let mut state = logging_state(&redactor);
    state.request_logging.raw_request_only = probe.raw_request == RawRequest::Only;
    let snapshots = Arc::new(Mutex::new(Vec::new()));
    let capture = capture(&snapshots);
    let env = prepost_env(&state, &redactor, &sink, Some(&capture));
    let outcome = call_outcome(&mut call, &fixtures, probe.supplied, &mut state, &env);
    let remaining = remaining_options(&call.optional_params).expect("the remaining options parse");
    Run {
        outcome,
        factory_requests: fixtures.factory.requests(),
        supplied_requests: supplied_requests(&fixtures),
        cohere_requests: fixtures.cohere_client.requests(),
        calls: fixtures.factory.calls(),
        regions: source.regions(),
        remaining,
        state,
        snapshots: take_snapshots(&snapshots),
    }
}

struct Fixture<'source> {
    factory: Factory,
    cohere_client: FixtureClient,
    supplied_client: Option<FixtureClient>,
    encoding: Counting,
    signing: AwsSigningExecutor,
    source: &'source Source,
    secrets: SecretLookup,
    region_worker: ManagedRegionWorker,
    clock: Arc<dyn Clock>,
}

fn fixtures<'source>(probe: &Probe, source: &'source Source) -> Fixture<'source> {
    let replies = Replies::queue(&probe.replies);
    Fixture {
        factory: Factory::new(replies.clone()),
        cohere_client: FixtureClient::new(replies.clone()),
        supplied_client: probe.supplied.map(|_kind| FixtureClient::new(replies)),
        encoding: Counting,
        signing: AwsSigningExecutor::default(),
        source,
        secrets: secret_lookup(),
        region_worker: ManagedRegionWorker::new().expect("the region worker starts"),
        clock: Arc::new(SteppingClock::new(probe.clock.clone(), now())),
    }
}

fn environment(probe: &Probe) -> BTreeMap<String, String> {
    probe
        .environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn probe_call<'call>(
    probe: &'call Probe,
    environment: &'call BTreeMap<String, String>,
    api_key: Option<&'call SecretString>,
    bearer: Option<&'call SecretString>,
) -> EmbeddingCall<'call> {
    EmbeddingCall {
        model: &probe.model,
        input: &probe.input,
        optional_params: raw_params(&probe.optional_params),
        api_base: probe.api_base.as_deref(),
        extra_headers: probe.extra_headers.as_deref(),
        api_key,
        timeout: probe.timeout.clone(),
        aembedding: probe.aembedding,
        drop_params: probe.drop_params,
        environment: EmbeddingEnvironment {
            variables: environment,
            runtime_endpoint: probe.runtime_endpoint.as_deref(),
            bearer_token: bearer,
        },
    }
}

fn capture(
    snapshots: &Arc<Mutex<Vec<Json>>>,
) -> impl Fn(&Json) -> Result<(), PrePostError> + 'static {
    let shared = Arc::clone(snapshots);
    move |details: &Json| -> Result<(), PrePostError> {
        shared
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(details.clone());
        Ok(())
    }
}

fn call_outcome(
    call: &mut EmbeddingCall<'_>,
    fixtures: &Fixture<'_>,
    supplied_kind: Option<ClientKind>,
    state: &mut LoggingState,
    env: &PrePostEnv<'_>,
) -> Result<EmbeddingCallResponse, EmbeddingCallError> {
    let supplied = fixtures
        .supplied_client
        .as_ref()
        .zip(supplied_kind)
        .map(|(client, kind)| SuppliedClient {
            kind,
            transport: client,
        });
    let dependencies = Dependencies {
        secrets: &fixtures.secrets,
        region_worker: &fixtures.region_worker,
        source: fixtures.source,
        factory: &fixtures.factory,
        transport: &fixtures.cohere_client,
        encoding: &fixtures.encoding,
        signing: &fixtures.signing,
        clock: Arc::clone(&fixtures.clock),
    };
    let mut log = LogContext { state, env };
    runtime().block_on(embeddings(call, dependencies, supplied, &mut log))
}

fn supplied_requests(fixtures: &Fixture<'_>) -> Vec<Recorded> {
    fixtures
        .supplied_client
        .as_ref()
        .map_or_else(Vec::new, FixtureClient::requests)
}

fn take_snapshots(snapshots: &Arc<Mutex<Vec<Json>>>) -> Vec<Json> {
    snapshots
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}
