use std::collections::VecDeque;
use std::convert::Infallible;
use std::future::Future;
use std::mem;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, LoggerFn, PrePostEnv,
};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::{DateTime, FixedOffset};
use futures_util::{Stream, StreamExt as _};
use http::{HeaderMap, HeaderName, HeaderValue};
use tokio::runtime::Builder;
use uuid::Uuid;

use crate::agentcore_stream::IdSource;
use crate::agentcore_stream::transport::{
    Chunks, HandlerKind, PostRequest, StreamHandler, StreamHead,
};
use crate::bedrock_http_stream::converse::{
    ConverseCall, make_sync_call, open_call as open_converse,
};
use crate::bedrock_http_stream::error::HttpStreamError;
use crate::bedrock_http_stream::invoke::{InvokeCall, make_call, open_call as open_invoke};
use crate::bedrock_http_stream::stream::{OpenedStream, StreamFailure};
use crate::bedrock_http_stream::{PendingOpen, StreamDeps};
use crate::bedrock_settings::{BedrockError, ResponseBody};
use crate::image_call::plan::LogContext;
use crate::invoke_events::{DecoderClock, InvokeChunk};

pub(super) const BASE: &str =
    "https://bedrock-runtime.us-east-1.amazonaws.com/model/amazon.titan/invoke";
pub(super) const DATA: &str = r#"{"inputText":"hi"}"#;
pub(super) const REPLY: &str = r#"{"output":{"message":{"role":"assistant","content":[{"text":"hello"}]}},"stopReason":"end_turn","usage":{"inputTokens":10,"outputTokens":5,"totalTokens":15}}"#;
// One generic chunk event: {"outputText":"first"}
pub(super) const FIRST: &str = "000000660000002aa8eb94520b3a6576656e742d747970650700056368756e6b0d3a6d6573736167652d747970650700056576656e747b226279746573223a2265794a7664585277645852555a586830496a6f695a6d6c796333516966513d3d227d0bc3a082";
const INIT: &str = r#"{"model":"bedrock/amazon.titan","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const SECONDS: u64 = 1_767_225_600;

pub(super) fn block_on<F>(future: F) -> F::Output
where
    F: Future,
{
    Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

fn fixed_id() -> Uuid {
    Uuid::from_u128(0x5EED_0000_u128)
}

fn fixed_seconds() -> i64 {
    i64::try_from(SECONDS).unwrap()
}

pub(super) fn now() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(SECONDS))
        .unwrap()
}

pub(super) fn header_map(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        headers.append(
            HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_str(value).unwrap(),
        );
    }
    headers
}

pub(super) fn sorted_pairs(headers: &HeaderMap) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = headers
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_owned(),
                value.to_str().unwrap_or_default().to_owned(),
            )
        })
        .collect();
    pairs.sort();
    pairs
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Recorded {
    pub(super) url: String,
    pub(super) headers: Vec<(String, String)>,
    pub(super) body: String,
    pub(super) stream: bool,
}

#[derive(Clone, Copy)]
pub(super) enum Refusal {
    ReadTimeout,
    TooLarge(usize),
    Closed,
}

impl Refusal {
    fn error(self) -> TransportError {
        match self {
            Self::ReadTimeout => TransportError::Rejected(HttpError::Timeout),
            Self::TooLarge(limit) => TransportError::BodyTooLarge { limit },
            Self::Closed => TransportError::Closed,
        }
    }
}

pub(super) struct Reply {
    pub(super) status: u16,
    pub(super) headers: Vec<(String, String)>,
    pub(super) reads: Vec<Result<Vec<u8>, HttpError>>,
    pub(super) refused: Option<Refusal>,
}

impl Reply {
    fn event_stream(reads: Vec<Result<Vec<u8>, HttpError>>) -> Self {
        Self {
            status: 200,
            headers: vec![(
                "content-type".to_owned(),
                "application/vnd.amazon.eventstream".to_owned(),
            )],
            reads,
            refused: None,
        }
    }

    pub(super) fn frames(reads: &[&str]) -> Self {
        Self::event_stream(
            reads
                .iter()
                .map(|read| Ok(hex::decode(read).unwrap()))
                .collect(),
        )
    }

    pub(super) fn buffered(body: &str) -> Self {
        Self {
            status: 200,
            headers: vec![("content-type".to_owned(), "application/json".to_owned())],
            reads: vec![Ok(body.as_bytes().to_vec())],
            refused: None,
        }
    }

    // A reply with no content type whose one transport read carries the given outcome.
    pub(super) fn reading(status: u16, read: Result<Vec<u8>, HttpError>) -> Self {
        let mut reply = Self::event_stream(vec![read]);
        reply.status = status;
        reply.headers.clear();
        reply
    }

    pub(super) fn refusing(refused: Refusal) -> Self {
        Self {
            status: 200,
            headers: Vec::new(),
            reads: Vec::new(),
            refused: Some(refused),
        }
    }
}

pub(super) struct Handler {
    status: u16,
    headers: Vec<(String, String)>,
    reads: Mutex<Vec<Result<Vec<u8>, HttpError>>>,
    refused: Option<Refusal>,
    posts: Mutex<Vec<Recorded>>,
    calls: AtomicUsize,
    polls: Arc<AtomicUsize>,
}

impl Handler {
    pub(super) fn new(reply: Reply) -> Self {
        let Reply {
            status,
            headers,
            reads,
            refused,
        } = reply;
        Self {
            status,
            headers,
            reads: Mutex::new(reads),
            refused,
            posts: Mutex::new(Vec::new()),
            calls: AtomicUsize::new(0),
            polls: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub(super) fn posts(&self) -> Vec<Recorded> {
        self.posts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn calls(&self) -> usize {
        self.calls.load(Ordering::Relaxed)
    }

    pub(super) fn polls(&self) -> usize {
        self.polls.load(Ordering::Relaxed)
    }
}

pub(super) struct Inputs {
    pub(super) handler: Arc<Handler>,
}

impl Inputs {
    pub(super) fn new(reply: Reply) -> Self {
        Self::with_handler(Arc::new(Handler::new(reply)))
    }

    pub(super) fn with_handler(handler: Arc<Handler>) -> Self {
        Self { handler }
    }

    pub(super) fn posts(&self) -> Vec<Recorded> {
        self.handler.posts()
    }

    pub(super) fn calls(&self) -> usize {
        self.handler.calls()
    }

    pub(super) fn polls(&self) -> usize {
        self.handler.polls()
    }
}

pub(super) struct Counter(Mutex<u128>);

impl Counter {
    pub(super) fn new() -> Self {
        Self(Mutex::new(0))
    }
}

impl IdSource for Counter {
    fn next_id(&self) -> Uuid {
        let mut next = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let id = Uuid::from_u128(*next);
        *next = next.saturating_add(1);
        id
    }
}

#[async_trait]
impl StreamHandler for Handler {
    fn kind(&self) -> HandlerKind {
        HandlerKind::Async
    }

    async fn post(&self, request: PostRequest) -> Result<StreamHead, TransportError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        if let Some(refused) = self.refused {
            return Err(refused.error());
        }
        self.posts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded {
                url: request.url.to_string(),
                headers: sorted_pairs(&request.headers),
                body: String::from_utf8(request.body).expect("a fixture request body is UTF-8"),
                stream: request.stream,
            });
        let mut headers = HeaderMap::new();
        for (name, value) in &self.headers {
            headers.append(
                HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_str(value).unwrap(),
            );
        }
        let reads = mem::take(&mut *self.reads.lock().unwrap_or_else(PoisonError::into_inner));
        Ok(StreamHead {
            status: self.status,
            headers,
            chunks: counted(reads, Arc::clone(&self.polls)),
        })
    }
}

struct CountingStream {
    items: VecDeque<Result<Vec<u8>, HttpError>>,
    polls: Arc<AtomicUsize>,
}

impl Stream for CountingStream {
    type Item = Result<Vec<u8>, HttpError>;

    fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.polls.fetch_add(1, Ordering::Relaxed);
        Poll::Ready(self.items.pop_front())
    }
}

fn counted(reads: Vec<Result<Vec<u8>, HttpError>>, polls: Arc<AtomicUsize>) -> Chunks {
    CountingStream {
        items: reads.into_iter().collect(),
        polls,
    }
    .boxed()
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Consume {
    None,
    First,
    All,
}

pub(super) enum Call<'call> {
    Invoke(InvokeCall<'call>),
    Converse(ConverseCall<'call>),
}

pub(super) struct Outcome {
    pub(super) error: Option<HttpStreamError<Infallible>>,
    pub(super) headers: Option<HeaderMap>,
    pub(super) chunks: Vec<InvokeChunk>,
    pub(super) failures: Vec<StreamFailure>,
    pub(super) logs: Vec<Json>,
}

impl Outcome {
    pub(super) fn provider(&self) -> Option<&BedrockError> {
        let Some(HttpStreamError::Provider { error, .. }) = &self.error else {
            return None;
        };
        Some(error.as_ref())
    }

    pub(super) fn provider_status(&self) -> Option<u16> {
        self.provider().map(|error| error.error.status_code)
    }

    pub(super) fn provider_message(&self) -> Option<&str> {
        self.provider().map(|error| error.error.message.as_str())
    }

    pub(super) fn provider_body(&self) -> Option<&ResponseBody> {
        self.provider().map(|error| &error.response_body)
    }

    pub(super) fn failure_statuses(&self) -> Vec<Option<u16>> {
        self.failures.iter().map(StreamFailure::status).collect()
    }

    pub(super) fn logged(&self, index: usize) -> &Json {
        &self.logs[index]
    }
}

pub(super) struct Quiet;

impl LogSink for Quiet {
    fn emit(&self, _record: LogRecord) {}
}

pub(super) fn logging_state(redactor: &Redactor) -> LoggingState {
    let init: LoggingInit = serde_json::from_str(INIT).unwrap();
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
    .unwrap()
}

pub(super) fn env_time() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2026-01-01T00:00:00+00:00").unwrap()
}

pub(super) fn messages() -> Json {
    serde_json::from_str(r#"[{"role":"user","content":"hi"}]"#).unwrap()
}

// The member a log entry names; every entry these tests read carries it.
pub(super) fn member<'json>(value: &'json Json, name: &str) -> &'json Json {
    match value {
        Json::Object(object) => object.get(name).expect("a present member"),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("an object")
        }
    }
}

pub(super) fn invoke<'call>(
    api_base: &'call str,
    data: &'call str,
    messages: &'call Json,
) -> InvokeCall<'call> {
    InvokeCall {
        model: "bedrock/amazon.titan",
        api_base,
        headers: header_map(&[("content-type", "application/json")]),
        data,
        signed_json_body: None,
        messages,
        fake_stream: false,
        json_mode: None,
        provider: None,
        sync: false,
    }
}

// Every converse case posts the shared fixture body to BASE; the one that varies its URL overrides api_base.
pub(super) fn converse(messages: &Json) -> ConverseCall<'_> {
    ConverseCall {
        model: "bedrock/anthropic.claude-3",
        api_base: BASE,
        headers: header_map(&[("content-type", "application/json")]),
        data: DATA,
        messages,
        json_mode: None,
        fake_stream: false,
    }
}

pub(super) fn run(call: Call<'_>, inputs: &Inputs, consume: Consume) -> Outcome {
    let handler: Arc<dyn StreamHandler> = Arc::clone(&inputs.handler);
    block_on(open_with(
        call,
        Some(Arc::clone(&handler)),
        handler,
        consume,
    ))
}

// No supplied client: the helper must post through the default handler it was given.
pub(super) fn run_default(call: Call<'_>, inputs: &Inputs, consume: Consume) -> Outcome {
    let handler: Arc<dyn StreamHandler> = Arc::clone(&inputs.handler);
    block_on(open_with(call, None, handler, consume))
}

// A supplied client and a default handler that answer differently, so picking the wrong one shows.
pub(super) fn run_supplied(
    call: Call<'_>,
    supplied: &Inputs,
    default: &Inputs,
    consume: Consume,
) -> Outcome {
    let supplied_handler: Arc<dyn StreamHandler> = Arc::clone(&supplied.handler);
    let default_handler: Arc<dyn StreamHandler> = Arc::clone(&default.handler);
    block_on(open_with(
        call,
        Some(supplied_handler),
        default_handler,
        consume,
    ))
}

// The body-lifetime test drives the same call inside its own runtime, so its timeouts cover the whole open.
pub(super) async fn open(
    call: Call<'_>,
    handler: Arc<dyn StreamHandler>,
    consume: Consume,
) -> Outcome {
    open_with(call, Some(Arc::clone(&handler)), handler, consume).await
}

async fn open_with(
    call: Call<'_>,
    supplied: Option<Arc<dyn StreamHandler>>,
    handler: Arc<dyn StreamHandler>,
    consume: Consume,
) -> Outcome {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let deps = deps_for(supplied, handler, &redactor);
    let pending = open_pending(&call, &deps).await;
    let recorded = Mutex::new(Vec::new());
    let opened = open_logged(&call, &deps, pending, &recorded);
    let mut outcome = Outcome {
        error: None,
        headers: None,
        chunks: Vec::new(),
        failures: Vec::new(),
        logs: Vec::new(),
    };
    consume_opened(opened, consume, &mut outcome).await;
    outcome.logs = mem::take(&mut *recorded.lock().unwrap_or_else(PoisonError::into_inner));
    outcome
}

async fn open_pending(
    call: &Call<'_>,
    deps: &StreamDeps<'_>,
) -> Result<PendingOpen, HttpStreamError<Infallible>> {
    match call {
        Call::Invoke(invoke_call) => open_invoke(invoke_call, deps).await,
        Call::Converse(converse_call) => open_converse(converse_call, deps).await,
    }
}

// The conversion, its log entry and the fake stream's body all run synchronously, as LiteLLM runs them.
fn open_logged<'deps>(
    call: &Call<'_>,
    deps: &StreamDeps<'deps>,
    pending: Result<PendingOpen, HttpStreamError<Infallible>>,
    recorded: &Mutex<Vec<Json>>,
) -> Result<OpenedStream<'deps>, HttpStreamError<Infallible>> {
    let mut state = logging_state(deps.redactor);
    let sink = Quiet;
    let logger = |details: &Json| -> Result<(), PrePostError> {
        recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(details.clone());
        Ok(())
    };
    let env = log_env(deps.redactor, &sink, &logger);
    let opened = pending?;
    let mut log = LogContext {
        state: &mut state,
        env: &env,
    };
    match call {
        Call::Invoke(invoke_call) => make_call(invoke_call, deps, opened, &mut log),
        Call::Converse(converse_call) => make_sync_call(converse_call, deps, opened, &mut log),
    }
}

async fn consume_opened(
    opened: Result<OpenedStream<'_>, HttpStreamError<Infallible>>,
    consume: Consume,
    outcome: &mut Outcome,
) {
    match opened {
        Ok(stream) => consume_stream(stream, consume, outcome).await,
        Err(error) => outcome.error = Some(error),
    }
}

async fn consume_stream(stream: OpenedStream<'_>, consume: Consume, outcome: &mut Outcome) {
    outcome.headers = Some(stream.headers);
    let mut chunks = stream.chunks;
    if consume == Consume::None {
        return;
    }
    while let Some(item) = chunks.next().await {
        match item {
            Ok(parsed) => outcome.chunks.push(parsed),
            Err(error) => outcome.failures.push(error),
        }
        if consume == Consume::First {
            return;
        }
    }
}

fn log_env<'env>(
    redactor: &'env Redactor,
    sink: &'env Quiet,
    logger: LoggerFn<'env>,
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
        now: env_time(),
    }
}

fn deps_for(
    supplied: Option<Arc<dyn StreamHandler>>,
    handler: Arc<dyn StreamHandler>,
    redactor: &Redactor,
) -> StreamDeps<'_> {
    StreamDeps {
        supplied,
        default: handler,
        ids: Arc::new(Counter::new()),
        clock: DecoderClock {
            next_id: fixed_id,
            now: fixed_seconds,
        },
        redactor,
        timeout: Duration::from_secs(600),
        now: now(),
    }
}
