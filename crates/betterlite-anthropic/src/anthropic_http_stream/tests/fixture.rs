#![cfg(test)]

use std::convert::Infallible;
use std::mem;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::python_json;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use futures_util::StreamExt as _;
use futures_util::stream;
use http::{HeaderMap, HeaderValue};
use indexmap::IndexMap;
use thiserror::Error;
use uuid::Uuid;

use crate::anthropic_http_stream::call::{StreamBuild, StreamCall, StreamLogger, StreamPostCall};
use crate::anthropic_http_stream::error::{PostFailure, StatusFailure};
use crate::anthropic_http_stream::transport::{StreamPost, StreamReply, StreamTransport};
use crate::stream_decode::TransportChunk;
use crate::stream_decode::error::ReceiveError;
use crate::stream_state::state::IdSource;
use crate::usage::ReasoningTokenCounter;

pub(crate) const API_BASE: &str = "https://api.anthropic.com/v1/messages";
pub(crate) const DATA: &str = r#"{"model":"claude-3-5-sonnet","stream":true}"#;
pub(crate) const FRAME: &str = r#"data: {"type":"ping"}"#;
pub(crate) const ERROR_TEXT: &str = r#"{"type":"error","error":{"type":"invalid_request_error"}}"#;
pub(crate) const SECRET: &str = "sk-ant-secret";
pub(crate) const TIMEOUT_MS: u64 = 1500;
pub(crate) const TIMEOUT: Duration = Duration::from_millis(TIMEOUT_MS);
pub(crate) const MAX_EVENT_BYTES: usize = 0x0010_0000;
pub(crate) const BODY_BYTES: usize = 0x0100_0000;

#[derive(Debug, Error)]
pub(crate) enum FixtureFailure {
    #[error("the transport exploded")]
    Unknown,
    #[error("the transport timed out")]
    Timeout,
    #[error("the fixture has no reply left")]
    Exhausted,
}

enum Verdict {
    Reply {
        status_code: u16,
        headers: HeaderMap,
        lines: Vec<String>,
    },
    Failed(PostFailure<FixtureFailure>),
}

pub(crate) struct FixtureTransport {
    verdict: Mutex<Option<Verdict>>,
    posts: Mutex<Vec<StreamPost>>,
    polled: Arc<AtomicBool>,
}

impl FixtureTransport {
    fn new(verdict: Verdict) -> Self {
        Self {
            verdict: Mutex::new(Some(verdict)),
            posts: Mutex::new(Vec::new()),
            polled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(crate) fn reply(status_code: u16, headers: HeaderMap, lines: &[&str]) -> Self {
        Self::new(Verdict::Reply {
            status_code,
            headers,
            lines: lines.iter().copied().map(str::to_owned).collect(),
        })
    }

    pub(crate) fn status_bytes(
        status_code: u16,
        body: Vec<u8>,
        response_headers: HeaderMap,
        exception_headers: Option<HeaderMap>,
    ) -> Self {
        Self::new(Verdict::Failed(PostFailure::Status(Box::new(
            StatusFailure {
                status_code,
                body,
                exception_headers,
                response_headers: Some(response_headers),
            },
        ))))
    }

    pub(crate) fn failure(failure: PostFailure<FixtureFailure>) -> Self {
        Self::new(Verdict::Failed(failure))
    }

    pub(crate) fn posts(&self) -> Vec<StreamPost> {
        mem::take(&mut *self.posts.lock().unwrap_or_else(PoisonError::into_inner))
    }

    pub(crate) fn polled(&self) -> bool {
        self.polled.load(Ordering::SeqCst)
    }

    fn take(&self) -> Option<Verdict> {
        self.verdict
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
}

impl StreamTransport for FixtureTransport {
    type Failure = FixtureFailure;

    async fn post(
        &self,
        post: StreamPost,
    ) -> Result<StreamReply<FixtureFailure>, PostFailure<FixtureFailure>> {
        self.posts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(post);
        let verdict = self
            .take()
            .ok_or(PostFailure::Unknown(FixtureFailure::Exhausted))?;
        let (status_code, headers, lines) = match verdict {
            Verdict::Failed(failure) => return Err(failure),
            Verdict::Reply {
                status_code,
                headers,
                lines,
            } => (status_code, headers, lines),
        };
        let flag = Arc::clone(&self.polled);
        let items = lines
            .into_iter()
            .map(|line| Ok(TransportChunk::Text(line)))
            .collect::<Vec<Result<TransportChunk, ReceiveError<FixtureFailure>>>>();
        let chunks = stream::iter(items)
            .inspect(move |_item| flag.store(true, Ordering::SeqCst))
            .boxed();
        Ok(StreamReply {
            status_code,
            headers,
            lines: chunks,
        })
    }
}

pub(crate) struct FixedIds;

impl IdSource for FixedIds {
    fn next_id(&self) -> Uuid {
        Uuid::from_u128(1)
    }
}

pub(crate) struct FixedCounter;

impl ReasoningTokenCounter for FixedCounter {
    type Failure = Infallible;

    fn count_reasoning_tokens(&self, _text: &str) -> Result<usize, Self::Failure> {
        Ok(0)
    }
}

pub(crate) struct Silent;

impl LogSink for Silent {
    fn emit(&self, _record: LogRecord) {}
}

pub(crate) struct SilentLogger;

impl StreamLogger for SilentLogger {
    fn post_call(&self, _call: StreamPostCall<'_>) {}
}

pub(crate) fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds")
}

pub(crate) fn logging_state(messages: &Json) -> LoggingState {
    let text = format!(
        r#"{{"model": "claude-3-5-sonnet", "call_type": "completion", "start_time": "2026-01-01T00:00:00+00:00", "litellm_call_id": "call-1", "function_id": "fn-1", "litellm_trace_id": "trace-1", "messages": {}, "litellm_params": {{}}, "request_logging": {{}}}}"#,
        python_json::dumps(messages)
    );
    let init = serde_json::from_str(&text).expect("the logging fixture parses");
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
    .expect("the logging state builds")
}

pub(crate) fn logging_env<'env>(redactor: &'env Redactor, sink: &'env Silent) -> PrePostEnv<'env> {
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
        logger_fn: None,
        now: DateTime::parse_from_rfc3339("2026-01-01T00:00:05+00:00")
            .expect("the fixture clock parses"),
    }
}

// A logger that runs the real post_call into a state the test reads afterwards
pub(crate) struct RecordingLogger {
    state: Mutex<LoggingState>,
    redactor: Redactor,
    sink: Silent,
}

impl RecordingLogger {
    pub(crate) fn new(messages: &Json) -> Self {
        Self {
            state: Mutex::new(logging_state(messages)),
            redactor: redactor(),
            sink: Silent,
        }
    }

    pub(crate) fn state(&self) -> MutexGuard<'_, LoggingState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl StreamLogger for RecordingLogger {
    fn post_call(&self, call: StreamPostCall<'_>) {
        let env = logging_env(&self.redactor, &self.sink);
        let mut state = self.state();
        // LiteLLM passes api_key="" and no headers, so the upstream credential never reaches a callback
        post_call(
            &mut state,
            &PostCall {
                original_response: Json::Text(call.original_response.to_owned()),
                input: call.input.clone(),
                additional_args: AdditionalArgs {
                    api_base: None,
                    headers: None,
                    complete_input_dict: Some(RawBody::Text(call.complete_input_dict.to_owned())),
                    request_str: None,
                    openai_sdk: None,
                    passthrough: IndexMap::new(),
                },
            },
            &env,
        );
    }
}

pub(crate) fn request_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    headers.insert("x-api-key", HeaderValue::from_static(SECRET));
    headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
    headers
}

pub(crate) fn response_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    headers.insert("x-request-id", HeaderValue::from_static("req-1"));
    headers.insert(
        "anthropic-ratelimit-requests-limit",
        HeaderValue::from_static("100"),
    );
    headers
}

pub(crate) fn exception_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("x-request-id", HeaderValue::from_static("req-2"));
    headers
}

pub(crate) fn header<'headers>(headers: &'headers HeaderMap, name: &str) -> Option<&'headers str> {
    let value = headers.get(name)?;
    value.to_str().ok()
}

pub(crate) fn messages() -> Json {
    let mut message = IndexMap::new();
    message.insert("role".to_owned(), Json::Text("user".to_owned()));
    message.insert("content".to_owned(), Json::Text("hi".to_owned()));
    Json::List(vec![Json::Object(message)])
}

pub(crate) fn reverse_map() -> IndexMap<String, String> {
    let mut map = IndexMap::new();
    map.insert("t_sanitized".to_owned(), "Original Name".to_owned());
    map
}

pub(crate) fn stream_call<'call>(
    data: &'call str,
    messages: &'call Json,
    reverse: &'call IndexMap<String, String>,
) -> StreamCall<'call> {
    StreamCall {
        api_base: API_BASE,
        headers: request_headers(),
        data,
        messages,
        timeout: TIMEOUT,
        json_mode: true,
        speed: Some("fast"),
        tool_name_reverse_map: reverse,
    }
}

pub(crate) fn stream_build(ids: &dyn IdSource) -> StreamBuild<'_, FixedCounter> {
    stream_build_with(ids, BODY_BYTES)
}

pub(crate) fn stream_build_with(
    ids: &dyn IdSource,
    max_error_body_bytes: usize,
) -> StreamBuild<'_, FixedCounter> {
    StreamBuild {
        ids,
        counter: FixedCounter,
        max_event_bytes: MAX_EVENT_BYTES,
        max_error_body_bytes,
    }
}

pub(crate) fn assert_no_post_call(state: &LoggingState) {
    assert!(
        state.model_call_details.get("original_response").is_none(),
        "a call that opens no stream leaves no post_call record"
    );
}

pub(crate) fn assert_post_call(state: &LoggingState, messages: &Json, response: &str, data: &str) {
    let details = &state.model_call_details;
    assert_eq!(
        details.get("original_response"),
        Some(&Json::Text(response.to_owned()))
    );
    assert_eq!(details.get("input"), Some(messages));
    let additional = match details.get("additional_args") {
        Some(Json::Object(fields)) => fields,
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_))
        | None => {
            panic!("post_call records additional_args")
        }
    };
    assert_eq!(
        additional.get("complete_input_dict"),
        Some(&Json::Text(data.to_owned()))
    );
    // The logger gets api_key="" and no headers, so the upstream credential never reaches a callback
    assert!(!python_json::dumps(&Json::Object(state.model_call_details.clone())).contains(SECRET));
}
