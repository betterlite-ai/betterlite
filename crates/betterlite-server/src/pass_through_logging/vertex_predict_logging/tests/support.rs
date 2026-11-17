#![cfg(test)]

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::types::{
    CallbackPolicy, DynamicCallbacks, RequestLoggingFlags,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamLoggingState, StreamSuccessCall, StreamSuccessDispatch,
};
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_models::decisions::Json;
use betterlite_redact::secret::Redactor;
use betterlite_vertex::vertex_relay_metadata::types::{
    CompletionCost, CompletionCostRequest, CompletionResponse,
};
use chrono::DateTime;
use http::{HeaderMap, StatusCode};
use indexmap::IndexMap;
use rust_decimal::Decimal;

use super::super::{VertexEmbedContentInput, VertexPredictInput, VertexPredictLogger};

pub(super) struct Fixture {
    pub(super) logging_obj: StreamLoggingObject,
    pub(super) url_route: String,
    pub(super) headers: HeaderMap,
    pub(super) body: Vec<u8>,
}

struct NoopDispatch;

#[async_trait]
impl StreamSuccessDispatch for NoopDispatch {
    async fn dispatch_success(&self, _call: StreamSuccessCall<'_>) {}
}

fn logging_init() -> LoggingInit {
    LoggingInit {
        model: String::new(),
        call_type: "pass_through_endpoint".to_owned(),
        start_time: DateTime::parse_from_rfc3339("2024-01-01T00:00:00+00:00")
            .expect("a valid timestamp"),
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

fn logging_object() -> StreamLoggingObject {
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

fn fixture_with(url_route: &str, body: Vec<u8>) -> Fixture {
    Fixture {
        logging_obj: logging_object(),
        url_route: url_route.to_owned(),
        headers: HeaderMap::new(),
        body,
    }
}

pub(super) fn fixture(url_route: &str, body: &str) -> Fixture {
    fixture_with(url_route, body.as_bytes().to_vec())
}

pub(super) fn raw_fixture(url_route: &str, body: &[u8]) -> Fixture {
    fixture_with(url_route, body.to_vec())
}

pub(super) fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("a JSON literal")
}

pub(super) fn kwargs(entries: &[(&str, Json)]) -> PassThroughLoggingKwargs {
    let mut collected = IndexMap::new();
    for (key, value) in entries {
        collected.insert((*key).to_owned(), value.clone());
    }
    PassThroughLoggingKwargs { entries: collected }
}

pub(super) fn predict_input(fixture: &Fixture) -> VertexPredictInput<'_> {
    VertexPredictInput {
        logging_obj: &fixture.logging_obj,
        url_route: &fixture.url_route,
        status: StatusCode::OK,
        response_headers: &fixture.headers,
        response_body: &fixture.body,
        kwargs: PassThroughLoggingKwargs::default(),
    }
}

pub(super) fn embed_input<'fixture>(
    fixture: &'fixture Fixture,
    request_body: Option<&'fixture Json>,
) -> VertexEmbedContentInput<'fixture> {
    VertexEmbedContentInput {
        logging_obj: &fixture.logging_obj,
        url_route: &fixture.url_route,
        status: StatusCode::OK,
        response_headers: &fixture.headers,
        response_body: &fixture.body,
        request_body,
        kwargs: PassThroughLoggingKwargs::default(),
    }
}

#[derive(Clone, Debug)]
pub(super) struct RecordedCost {
    pub(super) model: String,
    pub(super) provider: &'static str,
    pub(super) location: Option<String>,
    pub(super) response_model: Option<String>,
    pub(super) usage: Option<(i64, i64, i64)>,
}

pub(super) struct FixedCost {
    price: Option<Decimal>,
    calls: Mutex<Vec<RecordedCost>>,
}

impl FixedCost {
    fn new(price: Option<Decimal>) -> Self {
        Self {
            price,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn calls(&self) -> Vec<RecordedCost> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl CompletionCost for FixedCost {
    fn completion_cost(&self, request: &CompletionCostRequest<'_>) -> Option<Decimal> {
        let response: &CompletionResponse = request.response;
        let usage = response.usage.as_ref().map(|usage| {
            (
                usage.prompt_tokens,
                usage.completion_tokens,
                usage.total_tokens,
            )
        });
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedCost {
                model: request.model.to_owned(),
                provider: request.custom_llm_provider.as_str(),
                location: request.vertex_location.map(ToOwned::to_owned),
                response_model: response.model.clone(),
                usage,
            });
        self.price
    }
}

pub(super) fn priced() -> Arc<FixedCost> {
    Arc::new(FixedCost::new(Some(Decimal::new(7, 4))))
}

pub(super) fn unpriced() -> Arc<FixedCost> {
    Arc::new(FixedCost::new(None))
}

pub(super) fn logger_for(cost: &Arc<FixedCost>) -> VertexPredictLogger {
    let shared = Arc::clone(cost);
    VertexPredictLogger::new(shared)
}

pub(super) fn result_json(outcome: &HandlerOutcome) -> Json {
    outcome
        .result
        .clone()
        .expect("the handler returned a result")
        .into_json()
}

pub(super) fn field<'json>(value: &'json Json, name: &str) -> &'json Json {
    let Json::Object(fields) = value else {
        panic!("the value is an object");
    };
    fields.get(name).expect("the field is present")
}

pub(super) fn text(value: &Json) -> &str {
    let Json::Text(text) = value else {
        panic!("the value is a string");
    };
    text.as_str()
}

pub(super) fn integer(value: &Json) -> u64 {
    let Json::Number(number) = value else {
        panic!("the value is a number");
    };
    number.as_u64().expect("a whole number")
}

pub(super) fn number_text(value: &Json) -> String {
    let Json::Number(number) = value else {
        panic!("the value is a number");
    };
    number.to_string()
}

pub(super) fn list(value: &Json) -> &[Json] {
    let Json::List(items) = value else {
        panic!("the value is a list");
    };
    items.as_slice()
}

pub(super) fn logged_model(logging_obj: &StreamLoggingObject) -> String {
    logging_obj
        .read(&mut |state| state.model.clone())
        .expect("the logging state is present")
}

pub(super) fn logged_provider(logging_obj: &StreamLoggingObject) -> Option<String> {
    logging_obj.read(&mut |state| state.custom_llm_provider.clone())?
}

pub(super) fn logged_call_type(logging_obj: &StreamLoggingObject) -> String {
    logging_obj
        .read(&mut |state| state.call_type.clone())
        .expect("the logging state is present")
}

pub(super) fn details(logging_obj: &StreamLoggingObject) -> IndexMap<String, Json> {
    logging_obj
        .read(&mut |state| state.model_call_details.clone())
        .expect("the logging state is present")
}
