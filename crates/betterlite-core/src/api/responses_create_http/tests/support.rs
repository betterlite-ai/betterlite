use std::collections::VecDeque;
use std::fmt::Debug;
use std::future::{Future, ready};
use std::mem::take;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_context::agentic_callback::error::AgenticHookError;
use betterlite_context::agentic_callback::{
    AgenticCall, AgenticCallback, AgenticGate, AgenticLoopCall, AgenticOverrides,
};
use betterlite_context::agentic_loop_plan::{FollowupMessage, JsonMap};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_openai::response_events::{
    RawResponse, ResponsesReply, transform_response as transform_openai_response,
};
use betterlite_openai::response_input::ResponsesInput;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use http::HeaderMap as HttpHeaders;
use http::Method;
use http::header::{HeaderName, HeaderValue};
use indexmap::IndexMap;
use serde_json::value::RawValue;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context as LayerContext, Layer};
use tracing_subscriber::registry;
use url::Url;

use crate::api::responses_agentic_execution::executor::{
    AnthropicContinuation, ContinuationExecutor, ResponsesContinuation,
};
use crate::api::responses_compact_http::error::{ConfigFailure, Stage};
use crate::api::responses_compact_http::service::{
    ClientKind, ClientSource, SharedSession, SuppliedClient,
};
use crate::api::responses_create_http::config::{
    CreateConfig, CreateParams, CreateSignRequest, CreateTransformRequest, Headers, Signed,
};
use crate::api::responses_create_http::error::CreateError;
use crate::api::responses_create_http::service::{
    CreateCall, CreateDeps, CreateHooks, CreateOutcome, CreateRequest, CreateTransport, LogContext,
    SentReply, run,
};
use crate::api::responses_hook_policy::callbacks::{
    CallbackLists, DeploymentHookDispatcher, NamedCallbackResolver, PolicyCallback,
};
use crate::api::responses_stream_driver::source::PrefetchedSource;
use crate::api::vector_store_http_write::error::{BaseLlmException, ClientError, HeaderMap};

pub(super) fn json(text: &str) -> PromptJson {
    serde_json::from_str(text).expect("the test JSON")
}

pub(super) fn object(text: &str) -> Object {
    serde_json::from_str(text).expect("the test object")
}

pub(super) fn models(text: &str) -> Json {
    serde_json::from_str(text).expect("the test JSON")
}

pub(super) fn raw(text: &str) -> Box<RawValue> {
    RawValue::from_string(text.to_owned()).expect("the raw JSON")
}

pub(super) fn text_input(text: &str) -> ResponsesInput {
    ResponsesInput::Text(text.to_owned())
}

pub(super) fn params(text: &str) -> CreateParams {
    CreateParams::from_map(serde_json::from_str(text).expect("the test params"))
        .expect("the params")
}

pub(super) fn headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

pub(super) const RESP_A: &str = r#"{"id":"resp_a","created_at":1,"object":"response","status":"completed","model":"gpt-4o","output":[]}"#;

pub(super) struct Reply {
    pub(super) status: u16,
    pub(super) headers: HttpHeaders,
    pub(super) body: Vec<u8>,
}

impl Reply {
    pub(super) fn json(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: HttpHeaders::new(),
            body: body.as_bytes().to_vec(),
        }
    }

    pub(super) fn with_header(mut self, name: &'static str, value: &str) -> Self {
        self.headers.insert(
            HeaderName::from_static(name),
            HeaderValue::from_str(value).expect("the header value"),
        );
        self
    }
}

pub(super) enum Answer {
    Json(Reply),
    Sse(Vec<String>),
    Failure(TransportError),
}

#[derive(Clone)]
pub(super) struct Sent {
    pub(super) method: Method,
    pub(super) url: Url,
    pub(super) headers: HttpHeaders,
    pub(super) body: Vec<u8>,
    pub(super) stream: bool,
    pub(super) timeout: Duration,
}

impl Sent {
    pub(super) fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name)?.to_str().ok()
    }

    pub(super) fn body_json(&self) -> PromptJson {
        serde_json::from_slice(&self.body).expect("the request body is JSON")
    }
}

fn body_bytes(body: &RequestBody) -> Vec<u8> {
    match body {
        RequestBody::Json(bytes) => bytes.clone(),
        RequestBody::Bytes { data, .. } => data.clone(),
        RequestBody::Empty | RequestBody::Multipart { .. } => Vec::new(),
    }
}

type Answers = Arc<Mutex<VecDeque<Answer>>>;
type Requests = Arc<Mutex<Vec<Sent>>>;

#[derive(Clone)]
pub(super) struct Wire {
    answers: Answers,
    sent: Requests,
}

impl Wire {
    pub(super) fn new(answers: Vec<Answer>) -> Self {
        Self {
            answers: Arc::new(Mutex::new(VecDeque::from(answers))),
            sent: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(super) fn requests(&self) -> Vec<Sent> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl CreateTransport for Wire {
    fn send(
        &self,
        request: CreateRequest,
        stream: bool,
    ) -> impl Future<Output = Result<SentReply, TransportError>> + Send {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Sent {
                method: request.method,
                url: request.url.clone(),
                headers: request.headers.clone(),
                body: body_bytes(&request.body),
                stream,
                timeout: request.timeout,
            });
        let answer = self
            .answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .expect("one scripted answer per send");
        ready(match answer {
            Answer::Json(reply) => Ok(SentReply::Buffered(UpstreamResponse {
                status: reply.status,
                headers: reply.headers,
                body: reply.body,
            })),
            Answer::Sse(payloads) => {
                let items = payloads
                    .into_iter()
                    .map(|payload| Ok(payload.into_bytes()))
                    .collect();
                Ok(SentReply::Streaming(Box::new(PrefetchedSource::new(items))))
            }
            Answer::Failure(error) => Err(error),
        })
    }
}

struct Supplied {
    kind: ClientKind,
    wire: Wire,
}

pub(super) struct Source {
    pub(super) wire: Wire,
    pub(super) created: Mutex<Vec<(Option<bool>, Option<SharedSession>)>>,
    supplied: Option<Supplied>,
}

impl Source {
    pub(super) fn with(wire: Wire) -> Self {
        Self {
            wire,
            created: Mutex::new(Vec::new()),
            supplied: None,
        }
    }

    pub(super) fn with_supplied(wire: Wire, kind: ClientKind, supplied: Wire) -> Self {
        Self {
            wire,
            created: Mutex::new(Vec::new()),
            supplied: Some(Supplied {
                kind,
                wire: supplied,
            }),
        }
    }

    pub(super) fn creations(&self) -> Vec<(Option<bool>, Option<SharedSession>)> {
        self.created
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
        self.created
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((ssl_verify, shared_session));
        Ok(self.wire.clone())
    }
}

#[derive(Default)]
pub(super) struct Config {
    pub(super) fail: Option<Stage>,
    pub(super) signed_body: Option<Vec<u8>>,
    pub(super) validated: Mutex<Vec<Headers>>,
    pub(super) signed: Mutex<Vec<SignRecord>>,
    pub(super) transform_headers: Mutex<Vec<IndexMap<String, String>>>,
    pub(super) pre_call_details: Mutex<Option<IndexMap<String, Json>>>,
    pub(super) off_loop_sign: AtomicBool,
    pub(super) off_loop_transform: AtomicBool,
}

pub(super) struct SignRecord {
    pub(super) headers: Headers,
    pub(super) data: Object,
    pub(super) api_base: String,
    pub(super) api_key: Option<String>,
    pub(super) model: String,
    pub(super) stream: bool,
    pub(super) fake_stream: Option<Json>,
}

impl Config {
    pub(super) fn recorded_sign(&self) -> SignRecord {
        self.signed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop()
            .expect("a sign request")
    }

    pub(super) fn validated_headers(&self) -> Vec<Headers> {
        self.validated
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn transformed_headers(&self) -> Vec<IndexMap<String, String>> {
        self.transform_headers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    // The details as pre_call recorded them, before the response transform's post_call overwrites them
    pub(super) fn pre_call_record(&self) -> IndexMap<String, Json> {
        self.pre_call_details
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .expect("the response transform follows the pre-call")
    }

    fn record_pre_call(&self, state: &LoggingState) {
        let mut recorded = self
            .pre_call_details
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if recorded.is_none() {
            *recorded = Some(state.model_call_details.clone());
        }
    }
}

fn failure(message: &str) -> ConfigFailure {
    #[derive(Debug, thiserror::Error)]
    #[error("{0}")]
    struct StepFailure(String);

    ConfigFailure::new(StepFailure(message.to_owned()))
}

impl CreateConfig for Config {
    fn validate_environment(
        &self,
        mut headers: Headers,
        _model: &str,
        params: &CreateParams,
    ) -> Result<Headers, ConfigFailure> {
        self.validated
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(headers.clone());
        if let Some(api_key) = &params.api_key {
            use secrecy::ExposeSecret as _;
            headers.insert(
                String::from("Authorization"),
                format!("Bearer {}", api_key.expose_secret()),
            );
        }
        headers
            .entry(String::from("Content-Type"))
            .or_insert_with(|| String::from("application/json"));
        Ok(headers)
    }

    fn complete_url(&self, params: &CreateParams) -> Result<String, ConfigFailure> {
        let base = params
            .api_base
            .as_deref()
            .unwrap_or("https://api.openai.com/v1");
        Ok(format!("{}/responses", base.trim_end_matches('/')))
    }

    fn transform_request(
        &self,
        request: &CreateTransformRequest<'_>,
    ) -> Result<Object, ConfigFailure> {
        if self.fail == Some(Stage::Transform) {
            return Err(failure("the transform failed"));
        }
        let mut data = request.optional_params.clone();
        data.shift_remove("extra_headers");
        data.shift_remove("timeout");
        data.insert(
            String::from("model"),
            PromptJson::Text(request.model.to_owned()),
        );
        let text = serde_json::to_string(request.input).map_err(ConfigFailure::new)?;
        let input: PromptJson = serde_json::from_str(&text).map_err(ConfigFailure::new)?;
        data.insert(String::from("input"), input);
        Ok(data)
    }

    fn transform_request_off_loop(
        &self,
        request: &CreateTransformRequest<'_>,
    ) -> impl Future<Output = Result<Object, ConfigFailure>> + Send {
        self.off_loop_transform.store(true, Ordering::SeqCst);
        ready(self.transform_request(request))
    }

    fn normalize_request(&self, data: Object) -> Result<Object, ConfigFailure> {
        if self.fail == Some(Stage::Normalize) {
            return Err(failure("the normalize failed"));
        }
        Ok(data)
    }

    fn sign_request(&self, request: &CreateSignRequest<'_>) -> Result<Signed, ConfigFailure> {
        if self.fail == Some(Stage::Sign) {
            return Err(failure("the sign failed"));
        }
        self.signed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(SignRecord {
                headers: request.headers.clone(),
                data: request.request_data.clone(),
                api_base: request.api_base.to_owned(),
                api_key: request.api_key.map(|key| {
                    use secrecy::ExposeSecret as _;
                    key.expose_secret().to_owned()
                }),
                model: request.model.to_owned(),
                stream: request.stream,
                fake_stream: request.fake_stream.cloned(),
            });
        let mut headers = request.headers.clone();
        headers.insert(String::from("X-Signed"), String::from("yes"));
        Ok(Signed {
            headers,
            body: self.signed_body.clone(),
        })
    }

    fn sign_request_off_loop(
        &self,
        request: &CreateSignRequest<'_>,
    ) -> impl Future<Output = Result<Signed, ConfigFailure>> + Send {
        self.off_loop_sign.store(true, Ordering::SeqCst);
        ready(self.sign_request(request))
    }

    fn transform_response(
        &self,
        reply: &RawResponse<'_>,
        state: &mut LoggingState,
        env: &PrePostEnv<'_>,
    ) -> Result<ResponsesReply, ConfigFailure> {
        self.transform_headers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(reply.headers.clone());
        self.record_pre_call(state);
        if self.fail == Some(Stage::Response) {
            return Err(failure("the response transform failed"));
        }
        transform_openai_response(reply, env.now.timestamp(), state, env)
            .map_err(ConfigFailure::new)
    }

    fn error_class(&self, message: &str, status_code: u16, headers: HeaderMap) -> BaseLlmException {
        BaseLlmException {
            status_code,
            message: message.to_owned(),
            headers,
            status_code_is_synthesized: false,
        }
    }
}

pub(super) struct Sink;

impl LogSink for Sink {
    fn emit(&self, _record: LogRecord) {}
}

pub(super) struct Fixture {
    redactor: Redactor,
    sink: Sink,
    state: LoggingState,
    now: DateTime<FixedOffset>,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let text = concat!(
            r#"{"model":"gpt-4o","call_type":"responses","start_time":"2026-01-01T00:00:00+00:00","#,
            r#""litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#
        );
        let init: LoggingInit = serde_json::from_str(text).expect("the logging init");
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor");
        let now = DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00").expect("the clock");
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
        .expect("the logging state");
        Self {
            redactor,
            sink: Sink,
            state,
            now,
        }
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

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub(super) struct HookFailure {
    pub(super) message: String,
}

pub(super) struct Dispatcher {
    pub(super) patch: Option<IndexMap<String, Json>>,
    pub(super) calls: AtomicUsize,
}

impl Dispatcher {
    pub(super) fn quiet() -> Self {
        Self {
            patch: None,
            calls: AtomicUsize::new(0),
        }
    }

    pub(super) fn patching(patch: IndexMap<String, Json>) -> Self {
        Self {
            patch: Some(patch),
            calls: AtomicUsize::new(0),
        }
    }

    pub(super) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl DeploymentHookDispatcher for Dispatcher {
    type Error = HookFailure;

    async fn pre_call_deployment_hook(
        &self,
        _request: IndexMap<String, Json>,
        _call_type: &str,
    ) -> Result<Option<IndexMap<String, Json>>, Self::Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.patch.clone())
    }
}

// The messages each agentic hook was called with, in call order
#[derive(Debug, Clone, PartialEq)]
pub(super) enum ScriptedCall {
    Gate(Vec<PromptJson>),
    Loop(Vec<PromptJson>),
}

pub(super) struct AgenticRecorder {
    calls: Mutex<Vec<ScriptedCall>>,
    optional: Mutex<Vec<PromptJson>>,
}

impl AgenticRecorder {
    pub(super) fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            optional: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<ScriptedCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn optional(&self) -> Vec<PromptJson> {
        self.optional
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, call: ScriptedCall) {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(call);
    }
}

fn messages(messages: &[FollowupMessage]) -> Vec<PromptJson> {
    messages
        .iter()
        .map(|message| serde_json::from_str(message.0.get()).expect("the followup message parses"))
        .collect()
}

fn optional_params(params: &JsonMap) -> PromptJson {
    PromptJson::Object(
        params
            .iter()
            .map(|(key, value)| {
                let parsed = serde_json::from_str(value.get()).expect("the optional param parses");
                (key.clone(), parsed)
            })
            .collect(),
    )
}

pub(super) struct Scripted {
    pub(super) result: Option<Box<RawValue>>,
    recorder: Arc<AgenticRecorder>,
}

#[async_trait]
impl AgenticCallback for Scripted {
    fn callback_name(&self) -> &'static str {
        "scripted"
    }

    fn overrides(&self) -> AgenticOverrides {
        AgenticOverrides {
            should_run: true,
            run_loop: true,
            build_plan: false,
            post_response: false,
            cleanup: false,
        }
    }

    async fn should_run_agentic_loop(
        &self,
        call: &AgenticCall<'_>,
    ) -> Result<AgenticGate, AgenticHookError> {
        let gate = AgenticGate {
            run: true,
            tools: JsonMap::new(),
        };
        self.recorder
            .record(ScriptedCall::Gate(messages(call.messages)));
        Ok(gate)
    }

    async fn run_agentic_loop(
        &self,
        call: &AgenticLoopCall<'_>,
    ) -> Result<Option<Box<RawValue>>, AgenticHookError> {
        let recorded = messages(call.messages);
        self.recorder.record(ScriptedCall::Loop(recorded));
        self.recorder
            .optional
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(optional_params(call.optional_params));
        Ok(self.result.clone())
    }
}

pub(super) fn agentic_callback(
    result: Option<&str>,
    recorder: &Arc<AgenticRecorder>,
) -> PolicyCallback {
    PolicyCallback {
        name: String::from("scripted"),
        overrides_pre_hook: false,
        agentic: Some(Arc::new(Scripted {
            result: result.map(raw),
            recorder: Arc::clone(recorder),
        })),
    }
}

pub(super) fn pre_hook_callback() -> PolicyCallback {
    PolicyCallback {
        name: String::from("scripted-hook"),
        overrides_pre_hook: true,
        agentic: None,
    }
}

pub(super) const NO_CALLBACKS: CallbackLists<'static> = CallbackLists {
    global: &[],
    dynamic: &[],
};

pub(super) struct NoResolver;

pub(super) const NO_RESOLVER: NoResolver = NoResolver;

impl NamedCallbackResolver for NoResolver {
    fn resolve(&self, _name: &str) -> Option<PolicyCallback> {
        None
    }
}

pub(super) struct NoExecutor;

pub(super) const NO_EXECUTOR: NoExecutor = NoExecutor;

#[async_trait]
impl ContinuationExecutor for NoExecutor {
    type Error = HookFailure;

    async fn responses(&self, _call: &ResponsesContinuation) -> Result<Box<RawValue>, Self::Error> {
        Err(HookFailure {
            message: String::from("a continuation is not expected"),
        })
    }

    async fn anthropic_messages(
        &self,
        _call: &AnthropicContinuation,
    ) -> Result<Box<RawValue>, Self::Error> {
        Err(HookFailure {
            message: String::from("a continuation is not expected"),
        })
    }
}

pub(super) fn config_with(fail: Option<Stage>, signed_body: Option<Vec<u8>>) -> Config {
    Config {
        fail,
        signed_body,
        validated: Mutex::new(Vec::new()),
        signed: Mutex::new(Vec::new()),
        transform_headers: Mutex::new(Vec::new()),
        pre_call_details: Mutex::new(None),
        off_loop_sign: AtomicBool::new(false),
        off_loop_transform: AtomicBool::new(false),
    }
}

pub(super) fn create_deps<'deps>(
    config: &'deps Config,
    source: &'deps Source,
) -> CreateDeps<'deps, Config, Source> {
    CreateDeps {
        config,
        source,
        supplied: source.supplied.as_ref().map(|supplied| SuppliedClient {
            kind: supplied.kind,
            transport: &supplied.wire,
        }),
    }
}

// The logging state a finished run leaves, so a test reads what pre_call stored
pub(super) struct Logged {
    pub(super) outcome: Result<CreateOutcome, CreateError>,
    pub(super) details: IndexMap<String, Json>,
}

pub(super) async fn execute_logged(
    deps: &CreateDeps<'_, Config, Source>,
    mode: ClientKind,
    call: &CreateCall<'_>,
    hooks: &CreateHooks<'_, Dispatcher, NoExecutor>,
) -> Logged {
    let mut fixture = Fixture::new();
    let env = base_env(&fixture.redactor, &fixture.sink, fixture.now);
    let mut log = LogContext {
        state: &mut fixture.state,
        env: &env,
    };
    let outcome = run(deps, mode, call, hooks, &mut log).await;
    Logged {
        outcome,
        details: log.state.model_call_details.clone(),
    }
}

pub(super) async fn execute(
    deps: &CreateDeps<'_, Config, Source>,
    mode: ClientKind,
    call: &CreateCall<'_>,
    hooks: &CreateHooks<'_, Dispatcher, NoExecutor>,
) -> Result<CreateOutcome, CreateError> {
    let logged = execute_logged(deps, mode, call, hooks).await;
    logged.outcome
}

pub(super) fn hooks<'hooks>(
    callbacks: CallbackLists<'hooks>,
    dispatcher: &'hooks Dispatcher,
    resolver: &'hooks NoResolver,
) -> CreateHooks<'hooks, Dispatcher, NoExecutor> {
    CreateHooks {
        callbacks,
        resolver,
        dispatcher,
        executor: &NO_EXECUTOR,
        logging: None,
    }
}

pub(super) fn body_map(json: &PromptJson) -> &Object {
    match json {
        PromptJson::Object(fields) => fields,
        PromptJson::Null
        | PromptJson::Bool(_)
        | PromptJson::Number(_)
        | PromptJson::Text(_)
        | PromptJson::List(_) => panic!("the body is an object"),
    }
}

#[derive(Clone, Default)]
pub(super) struct CapturedDebug(Arc<Mutex<Vec<String>>>);

impl CapturedDebug {
    pub(super) fn take(&self) -> Vec<String> {
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

pub(super) struct DebugCapture {
    pub(super) logs: CapturedDebug,
    _guard: DefaultGuard,
}

// Captures every event on this thread, so the async client's debug line is observable
pub(super) fn capture_debug() -> DebugCapture {
    let logs = CapturedDebug::default();
    let guard = set_default(registry().with(logs.clone()));
    DebugCapture {
        logs,
        _guard: guard,
    }
}
