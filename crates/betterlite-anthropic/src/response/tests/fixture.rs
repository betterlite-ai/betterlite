use std::convert::Infallible;
use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::LoggingState;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use http::{HeaderMap, HeaderValue};
use indexmap::IndexMap;
use serde::de::DeserializeOwned;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::response::error::ResponseError;
use crate::response::wire::ResponseEnvelope;
use crate::response::{Logging, RawResponse, ResponseRequest, transform_response};
use crate::usage::ReasoningTokenCounter;

pub(crate) const MODEL: &str = "claude-sonnet-4-6";
pub(crate) const NOW: i64 = 1_767_225_600;
pub(crate) const CALLER_ID: &str = "chatcmpl-b580";
pub(crate) const UPSTREAM_ID: &str = "msg_upstream_b580";
pub(crate) const USAGE: &str = r#"{"input_tokens":10,"cache_creation_input_tokens":5,"cache_read_input_tokens":3,"output_tokens":4,"output_tokens_details":{"thinking_tokens":2}}"#;
pub(crate) const TEXT: &str = r#"[{"type":"text","text":"Hello"}]"#;

pub(crate) fn body(content: &str, stop_reason: &str) -> String {
    format!(
        r#"{{"id":"{UPSTREAM_ID}","type":"message","role":"assistant","model":"{MODEL}","content":{content},"stop_reason":"{stop_reason}","stop_sequence":null,"usage":{USAGE}}}"#
    )
}

pub(crate) fn response_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("content-type", HeaderValue::from_static("application/json"));
    headers.insert("x-request-id", HeaderValue::from_static("req-b580"));
    headers.insert(
        "anthropic-ratelimit-requests-limit",
        HeaderValue::from_static("100"),
    );
    headers
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

pub(crate) struct Scenario {
    id: String,
    status: u16,
    headers: HeaderMap,
    body: Vec<u8>,
    messages: Vec<Json>,
    optional_params: IndexMap<String, Json>,
    litellm_params: IndexMap<String, Json>,
    request_data: IndexMap<String, Json>,
    json_mode: Option<bool>,
    now: i64,
    disable_add_prefix_to_prompt: bool,
    with_choices: bool,
}

impl Scenario {
    pub(crate) fn new(body: &str) -> Self {
        Self {
            id: CALLER_ID.to_owned(),
            status: 200,
            headers: response_headers(),
            body: body.as_bytes().to_vec(),
            messages: parsed(r#"[{"role":"user","content":"hi"}]"#),
            optional_params: IndexMap::new(),
            litellm_params: IndexMap::new(),
            request_data: IndexMap::new(),
            json_mode: None,
            now: NOW,
            disable_add_prefix_to_prompt: false,
            with_choices: true,
        }
    }

    #[must_use]
    pub(crate) fn raw(mut self, body: &[u8]) -> Self {
        self.body = body.to_vec();
        self
    }

    #[must_use]
    pub(crate) fn status(mut self, status: u16) -> Self {
        self.status = status;
        self
    }

    #[must_use]
    pub(crate) fn messages(mut self, source: &str) -> Self {
        self.messages = parsed(source);
        self
    }

    #[must_use]
    pub(crate) fn optional_params(mut self, source: &str) -> Self {
        self.optional_params = parsed(source);
        self
    }

    #[must_use]
    pub(crate) fn litellm_params(mut self, source: &str) -> Self {
        self.litellm_params = parsed(source);
        self
    }

    #[must_use]
    pub(crate) fn json_mode(mut self, enabled: bool) -> Self {
        self.json_mode = Some(enabled);
        self
    }

    #[must_use]
    pub(crate) fn with_disabled_prefix(mut self) -> Self {
        self.disable_add_prefix_to_prompt = true;
        self
    }

    #[must_use]
    pub(crate) fn without_choices(mut self) -> Self {
        self.with_choices = false;
        self
    }

    pub(crate) fn run(&self) -> Outcome {
        let (guard, warnings) = capture_warnings();
        let redactor = redactor();
        let sink = Silent;
        let mut state = logging_state();
        let env = logging_env(&redactor, &sink);
        let request = ResponseRequest {
            messages: &self.messages,
            optional_params: &self.optional_params,
            litellm_params: &self.litellm_params,
            request_data: &self.request_data,
            json_mode: self.json_mode,
            now: self.now,
            disable_add_prefix_to_prompt: self.disable_add_prefix_to_prompt,
        };
        let raw = RawResponse {
            status: self.status,
            headers: &self.headers,
            body: &self.body,
        };
        let mut envelope = ResponseEnvelope::new(self.id.clone());
        if !self.with_choices {
            envelope.choices.clear();
        }
        let logging = Logging {
            state: &mut state,
            env: &env,
        };
        let result = transform_response(&mut envelope, &raw, &request, logging, &FixedCounter)
            .map(|()| envelope);
        drop(guard);
        Outcome {
            result,
            details: state.model_call_details,
            warnings: warnings.messages(),
        }
    }
}

pub(crate) struct Outcome {
    pub(crate) result: Result<ResponseEnvelope, ResponseError>,
    pub(crate) details: IndexMap<String, Json>,
    pub(crate) warnings: Vec<String>,
}

impl Outcome {
    pub(crate) fn envelope(self) -> ResponseEnvelope {
        self.result.expect("the transform should succeed")
    }

    pub(crate) fn error(self) -> ResponseError {
        self.result.expect_err("the transform should fail")
    }
}

pub(crate) fn parsed<T>(source: &str) -> T
where
    T: DeserializeOwned,
{
    serde_json::from_str(source).expect("the fixture JSON should parse")
}

fn redactor() -> Redactor {
    Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).expect("the redactor builds")
}

fn logging_state() -> LoggingState {
    let text = format!(
        r#"{{"model": "{MODEL}", "call_type": "completion", "start_time": "2026-01-01T00:00:00+00:00", "litellm_call_id": "call-1", "function_id": "fn-1", "litellm_trace_id": "trace-1", "messages": [], "litellm_params": {{}}, "request_logging": {{}}}}"#
    );
    LoggingState::new(
        parsed(&text),
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

fn logging_env<'env>(redactor: &'env Redactor, sink: &'env Silent) -> PrePostEnv<'env> {
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

#[derive(Clone, Default)]
struct Warnings(Arc<Mutex<Vec<String>>>);

#[derive(Default)]
struct Rendered(String);

impl Visit for Rendered {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl Warnings {
    fn messages(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl<Inner: Subscriber> Layer<Inner> for Warnings {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != Level::WARN {
            return;
        }
        let mut rendered = Rendered::default();
        event.record(&mut rendered);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(rendered.0);
    }
}

fn capture_warnings() -> (DefaultGuard, Warnings) {
    let warnings = Warnings::default();
    let subscriber = tracing_subscriber::registry().with(warnings.clone());
    (set_default(subscriber), warnings)
}

pub(crate) fn logged(details: &IndexMap<String, Json>) -> &Json {
    details
        .get("original_response")
        .expect("the raw reply should be logged")
}
