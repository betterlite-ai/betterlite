use std::future::Future;
use std::mem;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use aws_smithy_eventstream::frame::write_message_to;
use aws_smithy_types::event_stream::{Header, HeaderValue, Message};
use aws_smithy_types::str_bytes::StrBytes;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_core::rules::{RuleDecision, RuleEvaluator};
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
use betterlite_logging::logging_streams::assembly::StreamBuild;
use betterlite_logging::logging_streams::duration::DURATION_KEY;
use betterlite_models::controls::control_options::ControlOptions;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use betterlite_sdk::call_setup::RequestMap;
use betterlite_sdk::call_setup::correlation::CorrelationRestore;
use betterlite_sdk::chat_stream::config::REPEATED_CHUNK_LIMIT;
use betterlite_sdk::chat_stream::ports::{
    ChatStreamPorts, StreamClock, StreamFailure, StreamLogging,
};
use betterlite_streaming::stream_chunk_shaping::creation::StreamInstant;
use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;
use chrono::DateTime;
use futures_util::{StreamExt as _, stream};
use http::{HeaderMap, HeaderName, HeaderValue as HttpHeaderValue};
use indexmap::IndexMap;
use tokio::runtime::Builder;
use uuid::Uuid;

use crate::agentcore_stream::IdSource;
use crate::agentcore_stream::transport::{HandlerKind, PostRequest, StreamHandler, StreamHead};
use crate::bedrock_http_stream::StreamDeps;
use crate::image_call::plan::LogContext;
use crate::invoke_events::DecoderClock;
use crate::invoke_stream::timing::{ApiTiming, TimingCall};
use crate::invoke_stream::{
    InvokeStreamCall, StreamGlobals, StreamSetup, StreamTransport, WrappedInvokeStream,
    custom_stream_wrapper, stream_transport,
};

// The litellm params CustomStreamWrapper folds into its hidden params: the model's id and a provider api_base.
const INIT: &str = r#"{"model":"bedrock/amazon.titan","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1","litellm_params":{"model_info":{"id":"model-1"},"api_base":"https://bedrock.example.com"}}"#;
const NOW: &str = "2026-01-01T00:00:00+00:00";
const SECONDS: u64 = 1_767_225_600;
const TIMEOUT: Duration = Duration::from_secs(600);
const START: StreamInstant = StreamInstant {
    seconds: 2,
    timestamp: 2.0,
};
const LATER: StreamInstant = StreamInstant {
    seconds: 5,
    timestamp: 5.0,
};

pub(super) const BASE: &str = "https://bedrock-runtime.us-east-1.amazonaws.com/model/anthropic.claude-3-haiku-20240307-v1%3A0/invoke";

// {"outputText":"first"} in a chunk event: what the base projection decodes for a Titan model.
pub(super) const TITAN: &str = r#"{"outputText":"first"}"#;

// The Converse reply a fake stream reads when ai21 appears in the api_base; _transform_response takes this shape.
pub(super) const BUFFERED: &str = r#"{"output":{"message":{"content":[{"text":"hello"}],"role":"assistant"}},"stopReason":"end_turn","usage":{"inputTokens":1,"outputTokens":1,"totalTokens":2}}"#;

pub(super) struct Quiet;

impl LogSink for Quiet {
    fn emit(&self, _record: LogRecord) {}
}

pub(super) struct Counter(Mutex<u128>);

impl IdSource for Counter {
    fn next_id(&self) -> Uuid {
        let mut next = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let id = Uuid::from_u128(*next);
        *next = next.saturating_add(1);
        id
    }
}

pub(super) struct Logs(Mutex<Vec<Json>>);

impl Logs {
    pub(super) fn new() -> Self {
        Self(Mutex::new(Vec::new()))
    }

    fn record(&self, details: &Json) -> Result<(), PrePostError> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(details.clone());
        Ok(())
    }

    fn entries(&self) -> Vec<Json> {
        let mut entries = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        mem::take(&mut *entries)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Recorded {
    pub(super) url: String,
    pub(super) headers: Vec<(String, String)>,
    pub(super) body: Vec<u8>,
    pub(super) stream: bool,
}

type Reads = Mutex<Vec<Result<Vec<u8>, HttpError>>>;

pub(super) struct Fixture {
    kind: HandlerKind,
    status: u16,
    headers: HeaderMap,
    reads: Reads,
    posts: Mutex<Vec<Recorded>>,
}

impl Fixture {
    pub(super) fn new(kind: HandlerKind, headers: &[(&str, &str)], body: Vec<u8>) -> Arc<Self> {
        Self::scripted(kind, 200, headers, vec![Ok(body)])
    }

    pub(super) fn failing(
        kind: HandlerKind,
        headers: &[(&str, &str)],
        reads: Vec<Result<Vec<u8>, HttpError>>,
    ) -> Arc<Self> {
        Self::scripted(kind, 200, headers, reads)
    }

    pub(super) fn refusing(kind: HandlerKind) -> Arc<Self> {
        Self::scripted(kind, 500, &[], vec![Ok(Vec::new())])
    }

    fn scripted(
        kind: HandlerKind,
        status: u16,
        headers: &[(&str, &str)],
        reads: Vec<Result<Vec<u8>, HttpError>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            kind,
            status,
            headers: header_map(headers),
            reads: Mutex::new(reads),
            posts: Mutex::new(Vec::new()),
        })
    }

    pub(super) fn recorded(&self) -> Vec<Recorded> {
        self.posts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl StreamHandler for Fixture {
    fn kind(&self) -> HandlerKind {
        self.kind
    }

    async fn post(&self, request: PostRequest) -> Result<StreamHead, TransportError> {
        self.posts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded {
                url: request.url.as_str().to_owned(),
                headers: sorted_pairs(&request.headers),
                body: request.body,
                stream: request.stream,
            });
        let reads = mem::take(&mut *self.reads.lock().unwrap_or_else(PoisonError::into_inner));
        Ok(StreamHead {
            status: self.status,
            headers: self.headers.clone(),
            chunks: stream::iter(reads).boxed(),
        })
    }
}

// The event-stream frame a Bedrock chunk event arrives in, built the way the wire lays it out.
pub(super) fn chunk_frame(source: &str) -> Vec<u8> {
    let payload = format!("{{\"bytes\":\"{}\"}}", STANDARD.encode(source));
    let headers = vec![
        Header::new(":event-type".to_owned(), text("chunk")),
        Header::new(":message-type".to_owned(), text("event")),
    ];
    let message = Message::new_from_parts(headers, payload.into_bytes());
    let mut frame = Vec::new();
    write_message_to(&message, &mut frame).unwrap();
    frame
}

fn text(value: &str) -> HeaderValue {
    HeaderValue::String(StrBytes::from(value.to_owned()))
}

#[derive(Default)]
struct Fakes {
    clock: FakeClock,
    logging: Logging,
    timing: Timing,
    rules: NoRules,
    restore: NoRestore,
    request_data: RequestMap,
}

pub(super) struct Opened {
    pub(super) model: String,
    pub(super) custom_llm_provider: &'static str,
    pub(super) headers: HeaderMap,
    pub(super) logs: Vec<Json>,
    pub(super) chunks: Vec<StreamChunk>,
    pub(super) failure: Option<String>,
    pub(super) duration: Option<Json>,
    pub(super) timings: Vec<TimingRecord>,
    pub(super) logged_chunks: usize,
    pub(super) successes: Vec<Option<Json>>,
    pub(super) completions: usize,
}

// The wrapper's output after its stream is read to its end: identity, chunks, failure and the logging it did.
struct Collected {
    model: String,
    custom_llm_provider: &'static str,
    headers: HeaderMap,
    chunks: Vec<StreamChunk>,
    failure: Option<String>,
}

// One wrapper call over the case's fixtures, consumed to its end so every port's work shows.
pub(super) fn run(
    call: &InvokeStreamCall<'_>,
    supplied: Option<Arc<Fixture>>,
    default: Arc<Fixture>,
    sync: bool,
    stream_options: Option<Json>,
) -> Opened {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let deps = deps(supplied.as_ref().map(shared), shared(&default), &redactor);
    let logs = Logs::new();
    let sink = Quiet;
    let logger = |details: &Json| logs.record(details);
    let env = pre_post_env(&redactor, &sink, &logger);
    let mut state = logging_state(&redactor);
    state.stream_options = stream_options;
    let fakes = Fakes::default();
    let mut correlation = CorrelationContext::default();
    let transport = stream_transport(call, sync);
    let mut log = LogContext {
        state: &mut state,
        env: &env,
    };
    let collected = match compose(&transport, &deps, &fakes, &mut correlation, &mut log) {
        Ok(wrapped) => consume(wrapped),
        Err(message) => Collected {
            model: String::new(),
            custom_llm_provider: "",
            headers: HeaderMap::new(),
            chunks: Vec::new(),
            failure: Some(message),
        },
    };
    Opened {
        model: collected.model,
        custom_llm_provider: collected.custom_llm_provider,
        headers: collected.headers,
        chunks: collected.chunks,
        failure: collected.failure,
        logs: logs.entries(),
        duration: state.model_call_details.get(DURATION_KEY).cloned(),
        timings: fakes.timing.entries(),
        logged_chunks: fakes.logging.logged_chunks(),
        successes: fakes.logging.successes(),
        completions: fakes.logging.completions(),
    }
}

fn compose<'deps>(
    transport: &StreamTransport<'_>,
    deps: &StreamDeps<'deps>,
    fakes: &'deps Fakes,
    correlation: &'deps mut CorrelationContext,
    log: &mut LogContext<'_, '_>,
) -> Result<WrappedInvokeStream<'deps>, String> {
    let ports = ChatStreamPorts {
        clock: &fakes.clock,
        logging: &fakes.logging,
        cache: None,
        hooks: &[],
        request_data: &fakes.request_data,
        call_type: "completion",
        rules: &fakes.rules,
        restore: &fakes.restore,
        correlation,
        redactor: deps.redactor,
    };
    let setup = StreamSetup {
        ports,
        timing: &fakes.timing,
        globals: StreamGlobals {
            max_streaming_duration: None,
            repeated_chunk_limit: REPEATED_CHUNK_LIMIT,
            logging_disable: false,
        },
    };
    block_on(custom_stream_wrapper(transport, deps, setup, log)).map_err(|error| error.to_string())
}

fn consume(mut wrapped: WrappedInvokeStream<'_>) -> Collected {
    let model = wrapped.model.clone();
    let custom_llm_provider = wrapped.custom_llm_provider;
    let headers = wrapped.response_headers.clone();
    let mut chunks = Vec::new();
    loop {
        match block_on(wrapped.completion_stream.next()) {
            Ok(Some(chunk)) => chunks.push(chunk),
            Ok(None) => {
                return Collected {
                    model,
                    custom_llm_provider,
                    headers,
                    chunks,
                    failure: None,
                };
            }
            Err(error) => {
                return Collected {
                    model,
                    custom_llm_provider,
                    headers,
                    chunks,
                    failure: Some(error.to_string()),
                };
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct TimingRecord {
    pub(super) call_type: String,
    pub(super) duration: f64,
    pub(super) start_time: f64,
    pub(super) end_time: f64,
}

// time.time() reads two seconds at the wrapper call's entry and five after it, so a duration must subtract.
#[derive(Default)]
pub(super) struct FakeClock(AtomicUsize);

impl StreamClock for FakeClock {
    fn now(&self) -> StreamInstant {
        if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
            START
        } else {
            LATER
        }
    }
}

#[derive(Default)]
struct Logged {
    chunks: usize,
    successes: Vec<Option<Json>>,
    completions: usize,
}

#[derive(Default)]
pub(super) struct Logging(Mutex<Logged>);

impl Logging {
    fn logged_chunks(&self) -> usize {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).chunks
    }

    fn successes(&self) -> Vec<Option<Json>> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .successes
            .clone()
    }

    fn completions(&self) -> usize {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .completions
    }
}

impl StreamLogging for Logging {
    fn started(&self, _at: StreamInstant) {}

    fn log_chunk(&self, _chunk: &StreamChunk, _cache_hit: bool) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).chunks += 1;
    }

    fn completed(&self, _chunks: &[StreamChunk]) -> StreamBuild {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .completions += 1;
        StreamBuild::Empty
    }

    fn partial_usage(&self, _chunks: &[StreamChunk]) {}

    fn log_failure(&self, _failure: &StreamFailure) {}

    fn log_success(&self, response: Option<Json>, _cache_hit: bool) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .successes
            .push(response);
    }
}

#[derive(Default)]
pub(super) struct Timing(Mutex<Vec<TimingRecord>>);

impl Timing {
    fn entries(&self) -> Vec<TimingRecord> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ApiTiming for Timing {
    fn service_success(&self, call: &TimingCall<'_>) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(TimingRecord {
                call_type: call.call_type.to_owned(),
                duration: call.duration,
                start_time: call.start_time,
                end_time: call.end_time,
            });
    }
}

#[derive(Default)]
pub(super) struct NoRules;

impl RuleEvaluator for NoRules {
    fn evaluate(&self, _index: usize, _input: &str) -> Option<RuleDecision> {
        None
    }
}

#[derive(Default)]
pub(super) struct NoRestore;

impl CorrelationRestore for NoRestore {
    fn restore_correlation_context(&self, _context: &mut CorrelationContext) {}
}

pub(super) fn deps<'deps>(
    supplied: Option<Arc<dyn StreamHandler>>,
    default: Arc<dyn StreamHandler>,
    redactor: &'deps Redactor,
) -> StreamDeps<'deps> {
    StreamDeps {
        supplied,
        default,
        ids: Arc::new(Counter(Mutex::new(0))),
        clock: DecoderClock {
            next_id: fixed_id,
            now: fixed_seconds,
        },
        redactor,
        timeout: TIMEOUT,
        now: now(),
    }
}

pub(super) fn shared(fixture: &Arc<Fixture>) -> Arc<dyn StreamHandler> {
    Arc::<Fixture>::clone(fixture)
}

pub(super) fn call<'call>(
    model: &'call str,
    api_base: &'call str,
    data: &'call IndexMap<String, Json>,
    messages: &'call Json,
) -> InvokeStreamCall<'call> {
    InvokeStreamCall {
        model,
        api_base,
        headers: header_map(&[("content-type", "application/json")]),
        data,
        messages,
        json_mode: None,
        signed_json_body: None,
        control: ControlOptions::default(),
    }
}

pub(super) fn messages() -> Json {
    serde_json::from_str(r#"[{"role":"user","content":"hi"}]"#).unwrap()
}

pub(super) fn data() -> IndexMap<String, Json> {
    serde_json::from_str(r#"{"inputText": "hi", "max_tokens": 5}"#).unwrap()
}

// The text each chunk carries in its first choice; the synthesized finish chunk has no choice.
pub(super) fn contents(chunks: &[StreamChunk]) -> Vec<Option<String>> {
    chunks
        .iter()
        .map(|chunk| {
            chunk
                .choices
                .first()
                .and_then(|choice| choice.delta.content.clone())
        })
        .collect()
}

pub(super) fn member<'json>(value: &'json Json, name: &str) -> &'json Json {
    match value {
        Json::Object(object) => object.get(name).expect("a present member"),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("an object")
        }
    }
}

pub(super) fn header_map(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        headers.append(
            HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HttpHeaderValue::from_str(value).unwrap(),
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

fn logging_state(redactor: &Redactor) -> LoggingState {
    let init: LoggingInit = serde_json::from_str(INIT).unwrap();
    LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("trace"),
        Duration::default,
    )
    .unwrap()
}

fn pre_post_env<'env>(
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
        now: DateTime::parse_from_rfc3339(NOW).unwrap(),
    }
}

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

fn now() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(SECONDS))
        .unwrap()
}
