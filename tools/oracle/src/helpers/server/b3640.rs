mod fixture;
mod view;

mod tests;

use std::sync::{Arc, Mutex, PoisonError};

use betterlite_core::routes::CallType;
use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_logging_state::logger::PassThroughEndpointLogging;
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_logging::passthrough_stream_logging::types::{
    CollectedChunksCall, CollectedChunksHandler, EndpointType, HandlerOutcome,
};
use betterlite_models::decisions::Json;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::pass_through_logging::gemini_passthrough_logging::cost::{
    CostRequest, CostResponse, PassthroughCompletionCost, PassthroughCostError,
};
use betterlite_server::pass_through_logging::gemini_passthrough_logging::{
    GeminiPassthroughInput, GeminiPassthroughLogger,
};
use betterlite_server::pass_through_logging::vertex_stream_collection::VertexStreamCollector;
use chrono::{DateTime, FixedOffset};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Map, Value, json};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    operation: Operation,
    url_route: String,
    #[serde(default)]
    body: Value,
    #[serde(default)]
    request_body: Option<Map<String, Value>>,
    #[serde(default)]
    lines: Vec<String>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    GeminiHandler,
    GeminiChunks,
    VertexChunks,
}

#[derive(Clone, Debug)]
struct RecordedCost {
    model: String,
    provider: String,
    response_model: Option<String>,
    usage: Option<(i64, i64, i64)>,
    kind: &'static str,
    call_type: Option<&'static str>,
    vertex_location: Option<String>,
}

struct RecordingCost {
    price: Decimal,
    calls: Mutex<Vec<RecordedCost>>,
}

impl RecordingCost {
    const fn new(price: Decimal) -> Self {
        Self {
            price,
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl PassthroughCompletionCost for RecordingCost {
    fn completion_cost(&self, request: &CostRequest<'_>) -> Result<Decimal, PassthroughCostError> {
        let (kind, response_model, usage) = match &request.response {
            CostResponse::Completion(body) => ("completion", body_model(body), body_usage(body)),
            CostResponse::Video(video) => ("video", video.model.clone(), None),
        };
        let call_type = match request.call_type {
            CallType::CreateVideo => Some("create_video"),
            _other => None,
        };
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(RecordedCost {
                model: request.model.to_owned(),
                provider: request.custom_llm_provider.to_owned(),
                response_model,
                usage,
                kind,
                call_type,
                vertex_location: request.vertex_location.map(str::to_owned),
            });
        Ok(self.price)
    }
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let cost = Arc::new(RecordingCost::new(Decimal::new(125, 2)));
        let logging_obj = fixture::logging_object()?;
        let passthrough = PassThroughEndpointLogging {
            injected_log_dispatch: None,
        };
        let call = collected_call(input, &logging_obj, &passthrough)?;
        let outcome = match input.operation {
            Operation::GeminiHandler => gemini_handler(input, &cost, &logging_obj),
            Operation::GeminiChunks => gemini_chunks(&call, &cost),
            Operation::VertexChunks => vertex_chunks(&call, &cost),
        };
        outcome.map_or_else(
            |_error| Ok(json!({ "failed": true })),
            |handled| view::outcome_view(&handled, &logging_obj, &cost),
        )
    }
}

fn collected_call<'input>(
    input: &'input Input,
    logging_obj: &'input StreamLoggingObject,
    passthrough: &'input PassThroughEndpointLogging,
) -> Result<CollectedChunksCall<'input>, Box<HelperError>> {
    Ok(CollectedChunksCall {
        logging_obj,
        passthrough,
        url_route: &input.url_route,
        request_body: None,
        endpoint_type: EndpointType::Gemini,
        start_time: start_time()?,
        end_time: start_time()?,
        lines: &input.lines,
        model: input.model.as_deref(),
    })
}

fn start_time() -> Result<DateTime<FixedOffset>, Box<HelperError>> {
    DateTime::parse_from_rfc3339("2024-01-01T00:00:00+00:00").or_value_error()
}

fn gemini_handler(
    input: &Input,
    cost: &Arc<RecordingCost>,
    logging_obj: &StreamLoggingObject,
) -> Result<HandlerOutcome, Box<HelperError>> {
    let bytes = serde_json::to_vec(&input.body).or_value_error()?;
    let empty = Map::new();
    let request_body = input.request_body.as_ref().unwrap_or(&empty);
    let headers = http::HeaderMap::new();
    let call_cost = Arc::clone(cost);
    GeminiPassthroughLogger::new(call_cost)
        .gemini_passthrough_handler(GeminiPassthroughInput {
            logging_obj,
            url_route: &input.url_route,
            response_headers: &headers,
            response_body: &bytes,
            request_body,
            start_time: start_time()?,
            end_time: start_time()?,
            kwargs: PassThroughLoggingKwargs::default(),
        })
        .map_err(|error| view::helper_error("HandlerError", &error.to_string()))
}

fn gemini_chunks(
    call: &CollectedChunksCall<'_>,
    cost: &Arc<RecordingCost>,
) -> Result<HandlerOutcome, Box<HelperError>> {
    let call_cost = Arc::clone(cost);
    GeminiPassthroughLogger::new(call_cost)
        .handle(call)
        .map_err(|error| view::helper_error("HandlerError", &error.to_string()))
}

fn vertex_chunks(
    call: &CollectedChunksCall<'_>,
    cost: &Arc<RecordingCost>,
) -> Result<HandlerOutcome, Box<HelperError>> {
    let call_cost = Arc::clone(cost);
    VertexStreamCollector::new(call_cost)
        .handle(call)
        .map_err(|error| view::helper_error("HandlerError", &error.to_string()))
}

fn body_model(body: &Json) -> Option<String> {
    let Json::Object(fields) = body else {
        return None;
    };
    match fields.get("model") {
        Some(Json::Text(model)) => Some(model.clone()),
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
        count(usage, "prompt_tokens")?,
        count(usage, "completion_tokens")?,
        count(usage, "total_tokens")?,
    ))
}

fn count(fields: &IndexMap<String, Json>, name: &str) -> Option<i64> {
    match fields.get(name)? {
        Json::Number(number) => number.as_i64(),
        Json::Null | Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_) => None,
    }
}
