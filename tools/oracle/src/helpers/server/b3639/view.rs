use std::sync::PoisonError;

use betterlite_logging::passthrough_logging_state::dispatch::PassThroughLoggingKwargs;
use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_models::decisions::{Extra, Json};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use indexmap::IndexMap;
use serde_json::{Map, Value, json};

use super::RecordingCost;

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

fn field_or_null(fields: &Map<String, Value>, name: &str) -> Value {
    fields
        .get(name)
        .cloned()
        .unwrap_or(Value::Null)
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
