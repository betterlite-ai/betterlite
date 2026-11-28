use std::collections::{BTreeMap, VecDeque};
use std::fmt::Debug;
use std::future::{Future, ready};
use std::mem::take;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::base_builder;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::{Serving, serve};
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json as LoggedJson;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::response_events::{
    ApiKeySources, RawResponse, ResponsesProvider, ResponsesReply, WorkloadIdentity,
    get_complete_url as openai_complete_url, transform_response as openai_transform_response,
    validate_environment as openai_validate_environment,
};
use betterlite_openai::response_input::ResponsesInput;
use betterlite_openai::response_resources::request::{
    CompactRequest as ProviderCompactRequest, ResourceRequest, compact_request,
};
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use http::HeaderMap as HttpHeaders;
use http::HeaderValue;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context as LayerContext, Layer};
use tracing_subscriber::registry;

use crate::api::responses_compact_http::config::{
    CompactConfig, CompactParams, CompactTransformRequest, Headers, SignRequest, Signed,
};
use crate::api::responses_compact_http::error::{CompactError, ConfigFailure};
use crate::api::responses_compact_http::service::{
    ClientKind, ClientSource, CompactCall, CompactDeps, CompactRequest, CompactTransport,
    LogContext, SharedSession, prepare, send,
};
use crate::api::vector_store_http_write::error::{BaseLlmException, ClientError, HeaderMap};

pub(crate) const COMPACT_OK: &str = r#"{"id":"resp_compact","created_at":100,"object":"response","status":"completed","model":"gpt-4o","output":[]}"#;
pub(crate) const ITEM_INPUT: &str =
    r#"[{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}]"#;
pub(crate) const NORMALIZED_BODY: &str = r#"{"instructions":"short","model":"gpt-4o","input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}],"normalized":true}"#;

#[derive(Clone, Default)]
pub(crate) struct Events(Arc<Mutex<Vec<String>>>);

impl Events {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn push(&self, name: &str) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(name.to_owned());
    }

    pub(crate) fn recorded(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

pub(crate) struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

pub(crate) struct Fixture {
    redactor: Redactor,
    sink: Sink,
    pub(crate) state: LoggingState,
    now: DateTime<FixedOffset>,
    // Each run's pre-call record, taken before the response transform overwrites the state's additional_args
    calls: Arc<Mutex<Vec<LoggedJson>>>,
}

impl Fixture {
    pub(crate) fn new(call_type: &str) -> Self {
        let init: LoggingInit = serde_json::from_str(&format!(
            r#"{{"model":"gpt-4o","call_type":"{call_type}","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}}"#
        ))
        .expect("the logging fixture is valid");
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds");
        let now =
            DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").expect("the clock parses");
        let state = LoggingState::new(
            init,
            CorrelationSetup {
                context: &mut CorrelationContext::default(),
                settings: CorrelationSettings::default(),
            },
            &redactor,
            || String::from("generated"),
            Duration::default,
        )
        .expect("the logging state builds");
        Self {
            redactor,
            sink: Sink,
            state,
            now,
            calls: Arc::default(),
        }
    }

    pub(crate) async fn run<C, S>(
        &mut self,
        deps: &CompactDeps<'_, C, S>,
        mode: ClientKind,
        call: &CompactCall<'_>,
        events: &Events,
    ) -> Result<ResponsesReply, CompactError>
    where
        C: CompactConfig,
        S: ClientSource<Client: CompactTransport>,
    {
        let logger = |_details: &LoggedJson| -> Result<(), PrePostError> {
            events.push("log");
            Ok(())
        };
        let prepared = {
            let env = PrePostEnv {
                logger_fn: Some(&logger),
                ..base_env(&self.redactor, &self.sink, self.now)
            };
            prepare(
                deps,
                mode,
                call,
                &mut LogContext {
                    state: &mut self.state,
                    env: &env,
                },
            )
            .await?
        };
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(LoggedJson::Object(self.state.model_call_details.clone()));
        send(
            prepared,
            &mut LogContext {
                state: &mut self.state,
                env: &base_env(&self.redactor, &self.sink, self.now),
            },
        )
        .await
    }

    pub(crate) fn logged(&self, name: &str) -> Option<LoggedJson> {
        let LoggedJson::Object(details) = self.first_call()? else {
            return None;
        };
        let LoggedJson::Object(args) = details.get("additional_args")? else {
            return None;
        };
        args.get(name).cloned()
    }

    pub(crate) fn input(&self) -> Option<LoggedJson> {
        let LoggedJson::Object(details) = self.first_call()? else {
            return None;
        };
        details.get("input").cloned()
    }

    fn first_call(&self) -> Option<LoggedJson> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .first()
            .cloned()
    }
}

fn base_env<'env>(
    redactor: &'env Redactor,
    log: &'env Sink,
    now: DateTime<FixedOffset>,
) -> PrePostEnv<'env> {
    PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now,
    }
}

#[derive(Clone)]
pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) headers: HttpHeaders,
    pub(crate) body: Vec<u8>,
}

impl Reply {
    pub(crate) fn json(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: HttpHeaders::new(),
            body: body.as_bytes().to_vec(),
        }
    }

    pub(crate) fn bytes(status: u16, body: Vec<u8>) -> Self {
        Self {
            status,
            headers: HttpHeaders::new(),
            body,
        }
    }

    pub(crate) fn with_header(mut self, name: &'static str, value: &str) -> Self {
        self.headers.insert(
            name,
            HeaderValue::from_str(value).expect("the header value"),
        );
        self
    }
}

type RecordedRequests = Arc<Mutex<Vec<Arc<CompactRequest>>>>;

#[derive(Clone, Default)]
struct WireState {
    failure: Arc<Mutex<Option<TransportError>>>,
    requests: RecordedRequests,
}

#[derive(Clone)]
pub(crate) struct Wire {
    reply: Reply,
    state: WireState,
    events: Events,
}

impl Wire {
    pub(crate) fn replying(reply: Reply, events: &Events) -> Self {
        Self {
            reply,
            state: WireState::default(),
            events: events.clone(),
        }
    }

    pub(crate) fn failing(error: TransportError, events: &Events) -> Self {
        let wire = Self::replying(Reply::json(200, "{}"), events);
        *wire
            .state
            .failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(error);
        wire
    }

    pub(crate) fn requests(&self) -> Vec<Arc<CompactRequest>> {
        self.state
            .requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl CompactTransport for Wire {
    fn default_timeout(&self) -> Duration {
        Duration::from_secs(60)
    }

    fn send(
        &self,
        request: CompactRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        self.events.push("send");
        let failure = self
            .state
            .failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        self.state
            .requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Arc::new(request));
        if let Some(error) = failure {
            return ready(Err(error));
        }
        ready(Ok(UpstreamResponse {
            status: self.reply.status,
            headers: self.reply.headers.clone(),
            body: self.reply.body.clone(),
        }))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FailureStage {
    Validate,
    Transform,
    Normalize,
    Sign,
    Response,
}

#[derive(Debug, Error)]
#[error("scripted provider failure")]
pub(crate) struct Boom;

#[derive(Debug, Clone)]
pub(crate) struct SignRecord {
    pub(crate) api_base: String,
    pub(crate) model: Option<String>,
    pub(crate) api_key: Option<String>,
    pub(crate) optional_params: CompactParams,
    pub(crate) data: Object,
}

struct RecordingState {
    base: String,
    fail_at: Option<FailureStage>,
    transformed_url: Option<String>,
    signed: Mutex<VecDeque<Option<Vec<u8>>>>,
    signs: Mutex<Vec<SignRecord>>,
}

impl RecordingState {
    fn new(base: &str) -> Self {
        Self {
            base: base.to_owned(),
            fail_at: None,
            transformed_url: None,
            signed: Mutex::default(),
            signs: Mutex::default(),
        }
    }
}

enum Backend {
    Recording(RecordingState),
    OpenAi { environment: Option<SecretString> },
}

pub(crate) struct RecordingConfig {
    backend: Backend,
    events: Events,
    models: Mutex<Vec<String>>,
    errors: Mutex<Vec<(String, u16, HeaderMap)>>,
}

pub(crate) enum Script {
    Fail(FailureStage),
    TransformTo(String),
}

impl RecordingConfig {
    pub(crate) fn new(base: &str, events: &Events) -> Self {
        Self {
            backend: Backend::Recording(RecordingState::new(base)),
            events: events.clone(),
            models: Mutex::default(),
            errors: Mutex::default(),
        }
    }

    // A transform URL that was never parsed, as a config that builds it by formatting would return
    pub(crate) fn scripted(mut self, script: Script) -> Self {
        if let Backend::Recording(state) = &mut self.backend {
            match script {
                Script::Fail(stage) => state.fail_at = Some(stage),
                Script::TransformTo(url) => state.transformed_url = Some(url),
            }
        }
        self
    }

    pub(crate) fn queue_signed(&self, body: Option<Vec<u8>>) {
        if let Backend::Recording(state) = &self.backend {
            state
                .signed
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push_back(body);
        }
    }

    pub(crate) fn signs(&self) -> Vec<SignRecord> {
        match &self.backend {
            Backend::Recording(state) => state
                .signs
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
            Backend::OpenAi { .. } => Vec::new(),
        }
    }

    pub(crate) fn models(&self) -> Vec<String> {
        self.models
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn errors(&self) -> Vec<(String, u16, HeaderMap)> {
        self.errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

pub(crate) fn openai_config(environment: &str, events: &Events) -> RecordingConfig {
    RecordingConfig {
        backend: Backend::OpenAi {
            environment: Some(SecretString::from(environment)),
        },
        ..RecordingConfig::new("", events)
    }
}

fn fail(fail_at: Option<FailureStage>, stage: FailureStage) -> Result<(), ConfigFailure> {
    if fail_at == Some(stage) {
        return Err(ConfigFailure::new(Boom));
    }
    Ok(())
}

fn provider_transform(
    request: &CompactTransformRequest<'_>,
) -> Result<ResourceRequest, ConfigFailure> {
    let provider = ProviderCompactRequest {
        model: request.model,
        input: request.input,
        options: request.optional_params,
        provider: ProviderId::Openai,
        model_info: None,
    };
    compact_request(&provider, request.api_base).map_err(ConfigFailure::new)
}

impl CompactConfig for RecordingConfig {
    fn validate_environment(
        &self,
        mut headers: Headers,
        model: &str,
        params: &CompactParams,
    ) -> Result<Headers, ConfigFailure> {
        self.events.push("validate");
        self.models
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(model.to_owned());
        match &self.backend {
            Backend::Recording(state) => {
                fail(state.fail_at, FailureStage::Validate)?;
                headers.insert(
                    String::from("Authorization"),
                    String::from("Bearer sk-provider"),
                );
                headers.insert(
                    String::from("Content-Type"),
                    String::from("application/json"),
                );
                Ok(headers)
            }
            Backend::OpenAi { environment } => {
                let mut identity = NoIdentity;
                Ok(openai_validate_environment(
                    headers,
                    ApiKeySources {
                        litellm_params: params.api_key.as_ref(),
                        litellm: None,
                        openai: None,
                        environment: environment.as_ref(),
                    },
                    params.api_base.as_deref(),
                    ResponsesProvider::OpenAi,
                    &mut identity,
                ))
            }
        }
    }

    fn complete_url(&self, params: &CompactParams) -> Result<String, ConfigFailure> {
        self.events.push("url");
        match &self.backend {
            Backend::Recording(state) => Ok(state.base.clone()),
            Backend::OpenAi { .. } => Ok(openai_complete_url(
                params.api_base.as_deref(),
                None,
                None,
                None,
            )),
        }
    }

    fn transform_compact_request(
        &self,
        request: &CompactTransformRequest<'_>,
    ) -> Result<ResourceRequest, ConfigFailure> {
        self.events.push("transform");
        if let Backend::Recording(state) = &self.backend {
            fail(state.fail_at, FailureStage::Transform)?;
            if let Some(url) = &state.transformed_url {
                return Ok(ResourceRequest {
                    url: url.clone(),
                    payload: request.optional_params.clone(),
                });
            }
        }
        provider_transform(request)
    }

    fn normalize_request(&self, mut data: Object) -> Result<Object, ConfigFailure> {
        self.events.push("normalize");
        if let Backend::Recording(state) = &self.backend {
            fail(state.fail_at, FailureStage::Normalize)?;
            data.insert(String::from("normalized"), PromptJson::Bool(true));
        }
        Ok(data)
    }

    fn sign_request(&self, request: &SignRequest<'_>) -> Result<Signed, ConfigFailure> {
        self.events.push("sign");
        match &self.backend {
            Backend::Recording(state) => {
                fail(state.fail_at, FailureStage::Sign)?;
                state
                    .signs
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(SignRecord {
                        api_base: request.api_base.to_owned(),
                        model: request.model.map(ToOwned::to_owned),
                        api_key: request.api_key.map(|key| key.expose_secret().to_owned()),
                        optional_params: request.optional_params.clone(),
                        data: request.request_data.clone(),
                    });
                let mut headers = request.headers.clone();
                headers.insert(String::from("X-Signed"), String::from("yes"));
                let body = state
                    .signed
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .pop_front()
                    .flatten();
                Ok(Signed { headers, body })
            }
            Backend::OpenAi { .. } => Ok(Signed {
                headers: request.headers.clone(),
                body: None,
            }),
        }
    }

    fn sign_request_off_loop(
        &self,
        request: &SignRequest<'_>,
    ) -> impl Future<Output = Result<Signed, ConfigFailure>> + Send {
        if let Backend::Recording(_) = &self.backend {
            self.events.push("off_loop");
        }
        ready(self.sign_request(request))
    }

    fn transform_compact_response(
        &self,
        reply: &RawResponse<'_>,
        state: &mut LoggingState,
        env: &PrePostEnv<'_>,
    ) -> Result<ResponsesReply, ConfigFailure> {
        self.events.push("response");
        if let Backend::Recording(recording) = &self.backend {
            fail(recording.fail_at, FailureStage::Response)?;
        }
        openai_transform_response(reply, env.now.timestamp(), state, env)
            .map_err(ConfigFailure::new)
    }

    fn error_class(&self, message: &str, status_code: u16, headers: HeaderMap) -> BaseLlmException {
        self.errors
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((message.to_owned(), status_code, headers.clone()));
        BaseLlmException {
            status_code,
            message: message.to_owned(),
            headers,
            status_code_is_synthesized: false,
        }
    }
}

struct NoIdentity;

impl WorkloadIdentity for NoIdentity {
    fn bearer_token(
        &mut self,
        _api_key: Option<&SecretString>,
        _api_base: Option<&str>,
    ) -> Option<SecretString> {
        None
    }
}

type Calls = Vec<(Option<bool>, Option<SharedSession>)>;

pub(crate) struct Source {
    wire: Wire,
    calls: Arc<Mutex<Calls>>,
}

impl Source {
    pub(crate) fn new(wire: Wire) -> Self {
        Self {
            wire,
            calls: Arc::default(),
        }
    }

    pub(crate) fn calls(&self) -> Calls {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ClientSource for Source {
    type Client = Wire;

    fn client(
        &self,
        ssl_verify: Option<bool>,
        shared_session: Option<SharedSession>,
    ) -> Result<Wire, ClientError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((ssl_verify, shared_session));
        Ok(self.wire.clone())
    }
}

pub(crate) fn proxied_client(limits: TransportLimits) -> ProxiedClient {
    let policy = Arc::new(
        ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
            .expect("the proxy policy resolves"),
    );
    ProxiedClient::build(base_builder().http1_only(), &policy, limits)
        .expect("the proxied client builds")
}

pub(crate) const IMPATIENT: TransportLimits = TransportLimits {
    read_timeout: Duration::from_millis(100),
    ..TransportLimits::DEFAULT
};

pub(crate) struct ProxiedSource {
    client: ProxiedClient,
}

impl ProxiedSource {
    pub(crate) fn new(limits: TransportLimits) -> Self {
        Self {
            client: proxied_client(limits),
        }
    }
}

impl ClientSource for ProxiedSource {
    type Client = ProxiedClient;

    fn client(
        &self,
        _ssl_verify: Option<bool>,
        _shared_session: Option<SharedSession>,
    ) -> Result<ProxiedClient, ClientError> {
        Ok(self.client.clone())
    }
}

pub(crate) fn params(api_base: &str) -> CompactParams {
    CompactParams {
        api_base: Some(api_base.to_owned()),
        api_key: None,
        ssl_verify: None,
    }
}

pub(crate) fn keyed_params(api_base: &str, api_key: &str) -> CompactParams {
    CompactParams {
        api_base: Some(api_base.to_owned()),
        api_key: Some(SecretString::from(api_key)),
        ssl_verify: None,
    }
}

pub(crate) fn optional_params(json: &str) -> Object {
    serde_json::from_str(json).expect("the optional params fixture is valid")
}

pub(crate) fn extra_body(json: &str) -> Object {
    serde_json::from_str(json).expect("the extra body fixture is valid")
}

pub(crate) fn caller_headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(crate) fn text_input(text: &str) -> ResponsesInput {
    ResponsesInput::Text(text.to_owned())
}

pub(crate) fn items_input(json: &str) -> ResponsesInput {
    serde_json::from_str(json).expect("the input fixture is valid")
}

pub(crate) fn json(text: &str) -> PromptJson {
    serde_json::from_str(text).expect("the expected JSON parses")
}

pub(crate) fn logged(text: &str) -> LoggedJson {
    serde_json::from_str(text).expect("the expected log parses")
}

pub(crate) fn compact_deps<'deps, C, S>(
    config: &'deps C,
    source: &'deps S,
) -> CompactDeps<'deps, C, S>
where
    C: CompactConfig,
    S: ClientSource,
{
    CompactDeps {
        config,
        source,
        supplied: None,
    }
}

pub(crate) fn masked_headers(pairs: &[(&str, &str)]) -> HeaderMap {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), LoggedJson::Text((*value).to_owned())))
        .collect()
}

pub(crate) fn slow_server() -> Serving {
    serve(
        reply("200 OK", &[("content-type", "application/json")], "{}"),
        Duration::from_millis(500),
    )
    .expect("the fixture serves")
}

pub(crate) struct TestCall<'call> {
    pub(crate) model: &'call str,
    pub(crate) input: &'call ResponsesInput,
    pub(crate) optional_params: &'call Object,
    pub(crate) params: &'call CompactParams,
    pub(crate) extra_headers: Option<&'call Headers>,
    pub(crate) extra_body: Option<&'call Object>,
    pub(crate) timeout: Option<Duration>,
    pub(crate) shared_session: Option<SharedSession>,
}

impl<'call> TestCall<'call> {
    pub(crate) const fn new(
        model: &'call str,
        input: &'call ResponsesInput,
        optional_params: &'call Object,
        params: &'call CompactParams,
    ) -> Self {
        Self {
            model,
            input,
            optional_params,
            params,
            extra_headers: None,
            extra_body: None,
            timeout: None,
            shared_session: None,
        }
    }

    pub(crate) fn call(&self) -> CompactCall<'call> {
        CompactCall {
            model: self.model,
            input: self.input,
            optional_params: self.optional_params,
            params: self.params,
            extra_headers: self.extra_headers,
            extra_body: self.extra_body,
            timeout: self.timeout,
            shared_session: self.shared_session,
        }
    }
}

pub(crate) fn body_bytes(request: &CompactRequest) -> Option<&[u8]> {
    match &request.body {
        RequestBody::Json(body) => Some(body),
        RequestBody::Bytes { data, .. } => Some(data),
        RequestBody::Empty | RequestBody::Multipart { .. } => None,
    }
}

pub(crate) fn body_json(request: &CompactRequest) -> PromptJson {
    let body = body_bytes(request).expect("the request carries a JSON body");
    serde_json::from_slice(body).expect("the request body is JSON")
}

#[derive(Clone, Default)]
pub(crate) struct CapturedDebug(Arc<Mutex<Vec<String>>>);

impl CapturedDebug {
    pub(crate) fn take(&self) -> Vec<String> {
        let mut messages = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        take(&mut messages)
    }
}

#[derive(Default)]
struct Message(String);

impl Visit for Message {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            value.clone_into(&mut self.0);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl<Inner: Subscriber> Layer<Inner> for CapturedDebug {
    fn on_event(&self, event: &Event<'_>, _context: LayerContext<'_, Inner>) {
        let mut message = Message::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.0);
    }
}

pub(crate) struct DebugCapture {
    pub(crate) logs: CapturedDebug,
    _guard: DefaultGuard,
}

// Captures every event on this thread, so the async client's debug line is observable
pub(crate) fn capture_debug() -> DebugCapture {
    let logs = CapturedDebug::default();
    let guard = set_default(registry().with(logs.clone()));
    DebugCapture {
        logs,
        _guard: guard,
    }
}
