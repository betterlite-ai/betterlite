#![cfg(test)]

use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_core::routes::CallType;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_logging_state::logger::PassThroughEndpointLogging;
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamLoggingState, StreamSuccessCall, StreamSuccessDispatch,
};
use betterlite_logging::passthrough_stream_logging::types::{
    CollectedChunksCall, EndpointType, HandlerOutcome,
};
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use chrono::{DateTime, FixedOffset};
use http::HeaderMap;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt;
use tracing_subscriber::fmt::MakeWriter;

use super::super::cost::{
    CostRequest, CostResponse, PassthroughCompletionCost, PassthroughCostError,
};
use super::super::{GeminiPassthroughInput, GeminiPassthroughLogger};

pub(crate) struct Fixture {
    pub(crate) logging_obj: StreamLoggingObject,
    pub(crate) url_route: String,
    pub(crate) headers: HeaderMap,
    pub(crate) body: Vec<u8>,
    pub(crate) request_body: IndexMap<String, Json>,
}

struct NoopDispatch;

#[async_trait]
impl StreamSuccessDispatch for NoopDispatch {
    async fn dispatch_success(&self, _call: StreamSuccessCall<'_>) {}
}

pub(crate) fn fixture(url_route: &str, body: &str) -> Fixture {
    Fixture {
        logging_obj: logging_object(),
        url_route: url_route.to_owned(),
        headers: HeaderMap::new(),
        body: body.as_bytes().to_vec(),
        request_body: IndexMap::new(),
    }
}

pub(crate) fn gemini_input(fixture: &Fixture) -> GeminiPassthroughInput<'_> {
    GeminiPassthroughInput {
        logging_obj: &fixture.logging_obj,
        url_route: &fixture.url_route,
        response_headers: &fixture.headers,
        response_body: &fixture.body,
        request_body: &fixture.request_body,
        start_time: timestamp(),
        end_time: timestamp(),
        kwargs: PassThroughLoggingKwargs::default(),
    }
}

pub(crate) fn logger(cost: Arc<FakeCost>) -> GeminiPassthroughLogger {
    GeminiPassthroughLogger::new(cost)
}

pub(crate) fn timestamp() -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339("2024-01-01T00:00:00+00:00").expect("a valid timestamp")
}

fn logging_init() -> LoggingInit {
    LoggingInit {
        model: String::new(),
        call_type: "pass_through_endpoint".to_owned(),
        start_time: timestamp(),
        litellm_call_id: "call-1".to_owned(),
        function_id: "function-1".to_owned(),
        messages: None,
        stream: Some(false),
        litellm_trace_id: Some("trace-1".to_owned()),
        kwargs: IndexMap::new(),
        litellm_params: IndexMap::new(),
        callback_dynamic_params: IndexMap::new(),
        trusted_callback_vars: Vec::new(),
        standard_built_in_tools_params: IndexMap::new(),
        dynamic_callbacks: DynamicCallbacks::default(),
        applied_guardrails: None,
        request_logging: RequestLoggingFlags::default(),
        callback_policy: CallbackPolicy::default(),
    }
}

pub(crate) fn logging_object() -> StreamLoggingObject {
    let redactor = Redactor::new(16).expect("a valid redactor");
    let mut context = CorrelationContext::default();
    let setup = CorrelationSetup {
        context: &mut context,
        settings: CorrelationSettings::default(),
    };
    let logging = LoggingState::new(
        logging_init(),
        setup,
        &redactor,
        || "generated-trace".to_owned(),
        || Duration::ZERO,
    )
    .expect("a valid logging state");
    StreamLoggingObject {
        slot: Mutex::new(Some(StreamLoggingState {
            logging,
            correlation: CorrelationContext::default(),
        })),
        dispatch: Arc::new(NoopDispatch),
    }
}

pub(crate) struct CollectedFixture {
    pub(crate) logging_obj: StreamLoggingObject,
    passthrough: PassThroughEndpointLogging,
    pub(crate) url_route: String,
    pub(crate) lines: Vec<String>,
    pub(crate) model: Option<String>,
}

impl CollectedFixture {
    pub(crate) fn call(&self) -> CollectedChunksCall<'_> {
        CollectedChunksCall {
            logging_obj: &self.logging_obj,
            passthrough: &self.passthrough,
            url_route: &self.url_route,
            request_body: None,
            endpoint_type: EndpointType::Gemini,
            start_time: timestamp(),
            end_time: timestamp(),
            lines: &self.lines,
            model: self.model.as_deref(),
        }
    }
}

pub(crate) fn collected(url_route: &str, lines: &[&str], model: Option<&str>) -> CollectedFixture {
    CollectedFixture {
        logging_obj: logging_object(),
        passthrough: PassThroughEndpointLogging {
            injected_log_dispatch: None,
        },
        url_route: url_route.to_owned(),
        lines: lines.iter().map(|line| (*line).to_owned()).collect(),
        model: model.map(str::to_owned),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RecordedCost {
    pub(crate) model: String,
    pub(crate) provider: String,
    pub(crate) call_type: CallType,
    pub(crate) kind: &'static str,
    pub(crate) response_model: Option<String>,
    pub(crate) usage: Option<(i64, i64, i64)>,
    pub(crate) vertex_location: Option<String>,
}

pub(crate) struct FakeCost {
    price: Option<Decimal>,
    calls: Mutex<Vec<RecordedCost>>,
}

impl FakeCost {
    pub(crate) fn priced(price: Decimal) -> Arc<Self> {
        Arc::new(Self {
            price: Some(price),
            calls: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn unpriced() -> Arc<Self> {
        Arc::new(Self {
            price: None,
            calls: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn calls(&self) -> Vec<RecordedCost> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl PassthroughCompletionCost for FakeCost {
    fn completion_cost(&self, request: &CostRequest<'_>) -> Result<Decimal, PassthroughCostError> {
        let (kind, response_model, usage) = match &request.response {
            CostResponse::Completion(body) => {
                ("completion", body_text(body, "model"), body_usage(body))
            }
            CostResponse::Video(video) => ("video", video.model.clone(), None),
        };
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedCost {
                model: request.model.to_owned(),
                provider: request.custom_llm_provider.to_owned(),
                call_type: request.call_type,
                kind,
                response_model,
                usage,
                vertex_location: request.vertex_location.map(str::to_owned),
            });
        self.price.ok_or_else(|| PassthroughCostError::Unavailable {
            model: request.model.to_owned(),
        })
    }
}

fn body_text(body: &Json, name: &str) -> Option<String> {
    let Json::Object(fields) = body else {
        return None;
    };
    match fields.get(name) {
        Some(Json::Text(text)) => Some(text.clone()),
        _other => None,
    }
}

fn body_usage(body: &Json) -> Option<(i64, i64, i64)> {
    let Json::Object(fields) = body else {
        return None;
    };
    let Some(Json::Object(usage)) = fields.get("usage") else {
        return None;
    };
    Some((
        token(usage, "prompt_tokens")?,
        token(usage, "completion_tokens")?,
        token(usage, "total_tokens")?,
    ))
}

fn token(fields: &IndexMap<String, Json>, name: &str) -> Option<i64> {
    match fields.get(name)? {
        Json::Number(number) => number.as_i64(),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
    }
}

pub(crate) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("a JSON literal")
}

pub(crate) fn result_json(outcome: &HandlerOutcome) -> Json {
    outcome.result.clone().expect("a logged result").into_json()
}

pub(crate) fn field<'json>(value: &'json Json, name: &str) -> &'json Json {
    let Json::Object(fields) = value else {
        panic!("the value is an object");
    };
    fields.get(name).expect("the object carries the key")
}

pub(crate) fn text<'json>(value: &'json Json, name: &str) -> &'json str {
    match field(value, name) {
        Json::Text(text) => text,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            panic!("the field is text")
        }
    }
}

pub(crate) fn integer(value: &Json, name: &str) -> i64 {
    match field(value, name) {
        Json::Number(number) => number.as_i64().expect("an integer"),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => {
            panic!("the field is a number")
        }
    }
}

pub(crate) fn number_text(value: &Json) -> String {
    match value {
        Json::Number(number) => number.to_string(),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => {
            panic!("the value is a number")
        }
    }
}

pub(crate) fn as_text(value: &Json) -> &str {
    match value {
        Json::Text(text) => text,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            panic!("the value is text")
        }
    }
}

pub(crate) fn kwarg<'outcome>(outcome: &'outcome HandlerOutcome, name: &str) -> &'outcome Json {
    outcome
        .kwargs
        .entries
        .get(name)
        .expect("the kwargs carry the key")
}

pub(crate) fn logged_model(logging_obj: &StreamLoggingObject) -> String {
    logging_obj
        .read(&mut |state| state.model.clone())
        .expect("the logging state is present")
}

pub(crate) fn logged_provider(logging_obj: &StreamLoggingObject) -> Option<String> {
    logging_obj.read(&mut |state| state.custom_llm_provider.clone())?
}

pub(crate) fn kwargs(entries: &[(&str, Json)]) -> PassThroughLoggingKwargs {
    let mut built = PassThroughLoggingKwargs::default();
    for (name, value) in entries {
        built.entries.insert((*name).to_owned(), value.clone());
    }
    built
}

pub(crate) fn detail(logging_obj: &StreamLoggingObject, name: &str) -> Option<Json> {
    logging_obj
        .read(&mut |state| state.model_call_details.get(name).cloned())
        .flatten()
}

pub(crate) fn optional_param(logging_obj: &StreamLoggingObject, name: &str) -> Option<Json> {
    logging_obj
        .read(&mut |state| state.optional_params.get(name).cloned())
        .flatten()
}

#[derive(Clone, Default)]
pub(crate) struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    pub(crate) fn contents(&self) -> String {
        let bytes = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        str::from_utf8(&bytes)
            .expect("the captured log lines are UTF-8")
            .to_owned()
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for LogBuffer {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

pub(crate) fn capture_logs(level: tracing::Level) -> (LogBuffer, DefaultGuard) {
    let buffer = LogBuffer::default();
    let subscriber = fmt()
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .with_max_level(level)
        .with_writer(buffer.clone())
        .finish();
    (buffer, set_default(subscriber))
}
