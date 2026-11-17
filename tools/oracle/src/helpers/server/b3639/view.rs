use std::sync::PoisonError;

use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_models::decisions::{Extra, Json};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use indexmap::IndexMap;
use serde_json::{Map, Value, json};

use super::RecordingCost;

const DETAILS: &str = "details";
const PROMPT_DETAILS: &str = "prompt_tokens_details";
const USAGE_KEYS: [&str; 3] = ["prompt_tokens", "completion_tokens", "total_tokens"];
const DETAIL_KEYS: [&str; 4] = [
    "text_tokens",
    "audio_tokens",
    "image_tokens",
    "video_tokens",
];

struct Logged {
    model: String,
    provider: Option<String>,
    call_type: String,
    details: IndexMap<String, Json>,
}

pub(super) fn outcome_view(
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
        "kwargs": entry_view(&outcome.kwargs.entries)?,
        "logging": logging_view(logging_obj)?,
        "cost": cost_view(cost),
    }))
}

fn result_view(response: &PassThroughResponse) -> Result<Value, Box<HelperError>> {
    match response {
        PassThroughResponse::Embedding(value) => {
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
    let mut view = token_view(fields, &USAGE_KEYS);
    view.insert(DETAILS.to_owned(), details_view(fields.get(PROMPT_DETAILS)));
    Value::Object(view)
}

fn details_view(details: Option<&Value>) -> Value {
    let Some(Value::Object(fields)) = details else {
        return Value::Null;
    };
    Value::Object(token_view(fields, &DETAIL_KEYS))
}

fn token_view(fields: &Map<String, Value>, keys: &[&str]) -> Map<String, Value> {
    let mut view = Map::new();
    for key in keys {
        let value = fields.get(*key).cloned().unwrap_or(Value::Null);
        view.insert((*key).to_owned(), value);
    }
    view
}

const fn count(value: Option<&Json>) -> usize {
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

// The kwargs and the logging details hold the same three keys, so both views share one builder.
fn entry_view(fields: &Extra) -> Result<Value, Box<HelperError>> {
    Ok(json!({
        "model": entry(fields, "model")?,
        "custom_llm_provider": entry(fields, "custom_llm_provider")?,
        "response_cost": entry(fields, "response_cost")?,
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
    let mut view = Map::new();
    view.insert("model".to_owned(), Value::String(logged.model));
    view.insert(
        "custom_llm_provider".to_owned(),
        logged.provider.map_or(Value::Null, Value::String),
    );
    view.insert("call_type".to_owned(), Value::String(logged.call_type));
    view.insert("details".to_owned(), entry_view(&logged.details)?);
    Ok(Value::Object(view))
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
