use std::future::Future;
use std::mem;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
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
use betterlite_models::controls::control_options::ControlOptions;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use futures_util::{StreamExt as _, stream};
use http::{HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use tokio::runtime::Builder;
use uuid::Uuid;

use crate::agentcore_stream::IdSource;
use crate::agentcore_stream::transport::{HandlerKind, PostRequest, StreamHandler, StreamHead};
use crate::bedrock_http_stream::StreamDeps;
use crate::bedrock_http_stream::stream::ChunkStream;
use crate::image_call::plan::LogContext;
use crate::invoke_events::DecoderClock;
use crate::invoke_stream::{InvokeStreamCall, custom_stream_wrapper, stream_transport};

const INIT: &str = r#"{"model":"bedrock/amazon.titan","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const NOW: &str = "2026-01-01T00:00:00+00:00";
const SECONDS: u64 = 1_767_225_600;
const TIMEOUT: Duration = Duration::from_secs(600);

pub(super) const BASE: &str = "https://bedrock-runtime.us-east-1.amazonaws.com/model/anthropic.claude-3-haiku-20240307-v1%3A0/invoke";

// The first frame of a Bedrock event stream: {"outputText":"first"} in a chunk event.
pub(super) const FRAME: &str = "000000660000002aa8eb94520b3a6576656e742d747970650700056368756e6b0d3a6d6573736167652d747970650700056576656e747b226279746573223a2265794a7664585277645852555a586830496a6f695a6d6c796333516966513d3d227d0bc3a082";

pub(super) struct Quiet;

impl LogSink for Quiet {
    fn emit(&self, _record: LogRecord) {}
}

pub(super) struct Counter(Mutex<u128>);

impl Counter {
    fn new() -> Self {
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

pub(super) struct Fixture {
    kind: HandlerKind,
    headers: HeaderMap,
    body: Vec<u8>,
    posts: Mutex<Vec<Recorded>>,
}

impl Fixture {
    pub(super) fn new(kind: HandlerKind, headers: &[(&str, &str)], body: Vec<u8>) -> Arc<Self> {
        Arc::new(Self {
            kind,
            headers: header_map(headers),
            body,
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
        Ok(StreamHead {
            status: 200,
            headers: self.headers.clone(),
            chunks: stream::iter([Ok(self.body.clone())]).boxed(),
        })
    }
}

pub(super) struct Opened {
    pub(super) model: String,
    pub(super) custom_llm_provider: &'static str,
    pub(super) headers: HeaderMap,
    pub(super) logs: Vec<Json>,
}

// One wrapper call over the case's fixtures; the opened stream drops with the deps its decoder borrows.
pub(super) fn run(
    call: &InvokeStreamCall<'_>,
    supplied: Option<Arc<Fixture>>,
    default: Arc<Fixture>,
    sync: bool,
) -> Opened {
    let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
    let deps = deps(supplied.as_ref().map(shared), shared(&default), &redactor);
    let logs = Logs::new();
    let sink = Quiet;
    let logger = |details: &Json| logs.record(details);
    let env = pre_post_env(&redactor, &sink, &logger);
    let mut state = logging_state(&redactor);
    let mut log = LogContext {
        state: &mut state,
        env: &env,
    };
    let transport = stream_transport(call, sync);
    let wrapped = block_on(custom_stream_wrapper(&transport, &deps, &mut log)).unwrap();
    Opened {
        model: wrapped.model,
        custom_llm_provider: wrapped.custom_llm_provider,
        headers: wrapped.response_headers,
        logs: logs.entries(),
    }
}

// The stream a wrapper returned, for the one case that reads a decoded frame out of it.
pub(super) fn completion_stream<'deps>(
    call: &InvokeStreamCall<'_>,
    fixture: Arc<Fixture>,
    redactor: &'deps Redactor,
    sync: bool,
) -> ChunkStream<'deps> {
    let deps = deps(Some(shared(&fixture)), shared(&fixture), redactor);
    let logs = Logs::new();
    let sink = Quiet;
    let logger = |details: &Json| logs.record(details);
    let env = pre_post_env(redactor, &sink, &logger);
    let mut state = logging_state(redactor);
    let mut log = LogContext {
        state: &mut state,
        env: &env,
    };
    let transport = stream_transport(call, sync);
    block_on(custom_stream_wrapper(&transport, &deps, &mut log))
        .unwrap()
        .completion_stream
}

pub(super) fn deps<'deps>(
    supplied: Option<Arc<dyn StreamHandler>>,
    default: Arc<dyn StreamHandler>,
    redactor: &'deps Redactor,
) -> StreamDeps<'deps> {
    StreamDeps {
        supplied,
        default,
        ids: Arc::new(Counter::new()),
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
