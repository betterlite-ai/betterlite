mod tests;

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
use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_logging::passthrough_stream_logging::object::{
    StreamLoggingObject, StreamLoggingState, StreamSuccessCall, StreamSuccessDispatch,
};
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_models::decisions::{Extra, Json};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_redact::secret::Redactor;
use betterlite_server::pass_through_logging::vertex_predict_logging::{
    VertexEmbedContentInput, VertexPredictInput, VertexPredictLogger,
};
use betterlite_vertex::vertex_relay_metadata::types::{CompletionCost, CompletionCostRequest};
use chrono::DateTime;
use http::{HeaderMap, StatusCode};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    operation: Operation,
    url_route: String,
    status: u16,
    body: Value,
    #[serde(default)]
    request_body: Option<Value>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    Predict,
    PredictImage,
    PredictAudio,
    EmbedContent,
}

struct NoopDispatch;

#[async_trait]
impl StreamSuccessDispatch for NoopDispatch {
    async fn dispatch_success(&self, _call: StreamSuccessCall<'_>) {}
}

#[derive(Clone, Debug)]
struct Recorded {
    model: String,
    provider: &'static str,
    location: Option<String>,
    response_model: Option<String>,
}

struct RecordingCost {
    price: Decimal,
    calls: Mutex<Vec<Recorded>>,
}

impl RecordingCost {
    fn new(price: Decimal) -> Self {
        Self {
            price,
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl CompletionCost for RecordingCost {
    fn completion_cost(&self, request: &CompletionCostRequest<'_>) -> Option<Decimal> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Recorded {
                model: request.model.to_owned(),
                provider: request.custom_llm_provider.as_str(),
                location: request.vertex_location.map(ToOwned::to_owned),
                response_model: request.response.model.clone(),
            });
        Some(self.price)
    }
}

struct Logged {
    model: String,
    provider: Option<String>,
    call_type: String,
    details: IndexMap<String, Json>,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let cost = Arc::new(RecordingCost::new(Decimal::new(7, 4)));
        let logger = VertexPredictLogger::new(cost.clone());
        let logging_obj = logging_object()?;
        let bytes = serde_json::to_vec(&input.body).or_value_error()?;
        let request_body = request_body(input.request_body.as_ref())?;
        let status = StatusCode::from_u16(input.status).or_value_error()?;
        let headers = HeaderMap::new();
        let kwargs = PassThroughLoggingKwargs::default();
        let outcome = match input.operation {
            Operation::EmbedContent => {
                logger.handle_embed_content_response(VertexEmbedContentInput {
                    logging_obj: &logging_obj,
                    url_route: &input.url_route,
                    status,
                    response_headers: &headers,
                    response_body: &bytes,
                    request_body: request_body.as_ref(),
                    kwargs,
                })
            }
            Operation::Predict | Operation::PredictImage | Operation::PredictAudio => logger
                .handle_predict_response(VertexPredictInput {
                    logging_obj: &logging_obj,
                    url_route: &input.url_route,
                    status,
                    response_headers: &headers,
                    response_body: &bytes,
                    kwargs,
                }),
        };
        match outcome {
            Ok(outcome) => outcome_view(&outcome, &logging_obj, &cost),
            Err(_) => Ok(json!({ "failed": true })),
        }
    }
}

fn request_body(value: Option<&Value>) -> Result<Option<Json>, Box<HelperError>> {
    value
        .map(|value| serde_json::from_value(value.clone()).or_value_error())
        .transpose()
}

fn outcome_view(
    outcome: &HandlerOutcome,
    logging_obj: &StreamLoggingObject,
    cost: &RecordingCost,
) -> Result<Value, Box<HelperError>> {
    let result = match &outcome.result {
        Some(response) => result_view(response)?,
        None => Value::Null,
    };
    Ok(json!({
        "result": result,
        "kwargs": kwargs_view(&outcome.kwargs)?,
        "logging": logging_view(logging_obj)?,
        "cost": cost_view(cost),
    }))
}

fn result_view(response: &PassThroughResponse) -> Result<Value, Box<HelperError>> {
    match response {
        PassThroughResponse::Embedding(value) => embedding_view(value),
        PassThroughResponse::Image(value) => Ok(image_view(value)),
        PassThroughResponse::Standard(value) => {
            let body = serde_json::to_value(&value.response).or_value_error()?;
            Ok(json!({ "kind": "standard", "response": body }))
        }
        PassThroughResponse::ModelResponse(_)
        | PassThroughResponse::TextCompletion(_)
        | PassThroughResponse::VideoOperation(_)
        | PassThroughResponse::ResponsesApi(_)
        | PassThroughResponse::Transcription(_)
        | PassThroughResponse::VertexBatchRecord(_)
        | PassThroughResponse::VertexSearchWrapper(_) => Err(helper_error(
            "UnexpectedResult",
            "the predict handler returned another response kind",
        )),
    }
}

fn embedding_view(value: &Json) -> Result<Value, Box<HelperError>> {
    let Json::Object(fields) = value else {
        return Err(helper_error(
            "UnexpectedResult",
            "the embedding reply is no object",
        ));
    };
    Ok(json!({
        "kind": "embedding",
        "model": entry(fields, "model")?,
        "data": count(fields.get("data")),
        "usage": usage_view(&entry(fields, "usage")?),
    }))
}

fn image_view(value: &Json) -> Value {
    let data = match value {
        Json::Object(fields) => count(fields.get("data")),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => 0,
    };
    json!({ "kind": "image", "data": data })
}

fn usage_view(usage: &Value) -> Value {
    let Value::Object(fields) = usage else {
        return Value::Null;
    };
    json!({
        "prompt_tokens": field_or_null(fields, "prompt_tokens"),
        "completion_tokens": field_or_null(fields, "completion_tokens"),
        "total_tokens": field_or_null(fields, "total_tokens"),
        "details": details_view(fields.get("prompt_tokens_details")),
    })
}

fn details_view(details: Option<&Value>) -> Value {
    let Some(Value::Object(fields)) = details else {
        return Value::Null;
    };
    json!({
        "text_tokens": field_or_null(fields, "text_tokens"),
        "audio_tokens": field_or_null(fields, "audio_tokens"),
        "image_tokens": field_or_null(fields, "image_tokens"),
        "video_tokens": field_or_null(fields, "video_tokens"),
    })
}

fn field_or_null(fields: &serde_json::Map<String, Value>, name: &str) -> Value {
    fields.get(name).cloned().unwrap_or(Value::Null)
}

fn count(value: Option<&Json>) -> usize {
    let Some(Json::List(items)) = value else {
        return 0;
    };
    items.len()
}

fn entry(fields: &Extra, name: &str) -> Result<Value, Box<HelperError>> {
    fields.get(name).map_or(Ok(Value::Null), |value| {
        serde_json::to_value(value).or_value_error()
    })
}

fn kwargs_view(kwargs: &PassThroughLoggingKwargs) -> Result<Value, Box<HelperError>> {
    Ok(json!({
        "model": entry(&kwargs.entries, "model")?,
        "custom_llm_provider": entry(&kwargs.entries, "custom_llm_provider")?,
        "response_cost": entry(&kwargs.entries, "response_cost")?,
    }))
}

fn logging_view(logging_obj: &StreamLoggingObject) -> Result<Value, Box<HelperError>> {
    let Some(logged) = logging_obj.read(&mut |state| Logged {
        model: state.model.clone(),
        provider: state.custom_llm_provider.clone(),
        call_type: state.call_type.clone(),
        details: state.model_call_details.clone(),
    }) else {
        return Err(helper_error(
            "LoggingState",
            "the logging object holds no state",
        ));
    };
    Ok(json!({
        "model": logged.model,
        "custom_llm_provider": logged.provider,
        "call_type": logged.call_type,
        "details": {
            "model": entry(&logged.details, "model")?,
            "custom_llm_provider": entry(&logged.details, "custom_llm_provider")?,
            "response_cost": entry(&logged.details, "response_cost")?,
        },
    }))
}

fn cost_view(cost: &RecordingCost) -> Value {
    let calls = cost.calls.lock().unwrap_or_else(PoisonError::into_inner);
    let rendered: Vec<Value> = calls
        .iter()
        .map(|call| {
            json!({
                "model": call.model,
                "custom_llm_provider": call.provider,
                "vertex_location": call.location,
                "response_model": call.response_model,
            })
        })
        .collect();
    Value::Array(rendered)
}

fn logging_init() -> Result<LoggingInit, Box<HelperError>> {
    Ok(LoggingInit {
        model: String::new(),
        call_type: "pass_through_endpoint".to_owned(),
        start_time: DateTime::parse_from_rfc3339("2024-01-01T00:00:00+00:00").or_value_error()?,
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
    })
}

fn logging_object() -> Result<StreamLoggingObject, Box<HelperError>> {
    let redactor = Redactor::new(16).or_value_error()?;
    let mut context = CorrelationContext::default();
    let setup = CorrelationSetup {
        context: &mut context,
        settings: CorrelationSettings::default(),
    };
    let logging = LoggingState::new(
        logging_init()?,
        setup,
        &redactor,
        || "generated-trace".to_owned(),
        || Duration::ZERO,
    )
    .or_value_error()?;
    Ok(StreamLoggingObject {
        slot: Mutex::new(Some(StreamLoggingState {
            logging,
            correlation: CorrelationContext::default(),
        })),
        dispatch: Arc::new(NoopDispatch),
    })
}

fn helper_error(class: &str, message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
