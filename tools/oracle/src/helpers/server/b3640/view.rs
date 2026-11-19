use std::sync::PoisonError;

use betterlite_logging::passthrough_logging_state::response::PassThroughResponse;
use betterlite_logging::passthrough_stream_logging::object::StreamLoggingObject;
use betterlite_logging::passthrough_stream_logging::types::HandlerOutcome;
use betterlite_models::decisions::{Extra, Json};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use indexmap::IndexMap;
use serde_json::{Map, Value, json};

use super::RecordingCost;

const DETAIL_KEYS: [&str; 3] = ["model", "custom_llm_provider", "response_cost"];
const USAGE_KEYS: [&str; 3] = ["prompt_tokens", "completion_tokens", "total_tokens"];

pub(super) fn outcome_view(
    outcome: &HandlerOutcome,
    logging_obj: &StreamLoggingObject,
    cost: &RecordingCost,
) -> Result<Value, Box<HelperError>> {
    Ok(json!({
        "result": result_view(outcome.result.as_ref())?,
        "kwargs": entry_view(&outcome.kwargs.entries)?,
        "logging": logging_view(logging_obj)?,
        "cost": cost_view(cost),
    }))
}

fn result_view(response: Option<&PassThroughResponse>) -> Result<Value, Box<HelperError>> {
    let Some(result) = response else {
        return Ok(Value::Null);
    };
    let body = serde_json::to_value(result.clone().into_json()).or_value_error()?;
    let kind = if body.get("object").and_then(Value::as_str) == Some("video") {
        "video"
    } else {
        "chat"
    };
    Ok(json!({
        "kind": kind,
        "id": body.get("id").cloned().unwrap_or(Value::Null),
        "model": body.get("model").cloned().unwrap_or(Value::Null),
        "status": body.get("status").cloned().unwrap_or(Value::Null),
        "choices": choices_view(body.get("choices")),
        "usage": usage_view(body.get("usage")),
    }))
}

fn choices_view(choices: Option<&Value>) -> Value {
    let Some(Value::Array(rows)) = choices else {
        return Value::Array(Vec::new());
    };
    let rendered: Vec<Value> = rows
        .iter()
        .map(|row| {
            let content = row
                .get("message")
                .and_then(|message| message.get("content"))
                .cloned()
                .unwrap_or(Value::Null);
            json!({
                "finish_reason": row.get("finish_reason").cloned().unwrap_or(Value::Null),
                "content": content,
            })
        })
        .collect();
    Value::Array(rendered)
}

fn usage_view(usage: Option<&Value>) -> Value {
    let Some(Value::Object(fields)) = usage else {
        return Value::Null;
    };
    if !fields.contains_key("prompt_tokens") {
        return Value::Null;
    }
    let counts: Map<String, Value> = USAGE_KEYS
        .iter()
        .map(|key| {
            let value = fields.get(*key).cloned().unwrap_or(Value::Null);
            ((*key).to_owned(), value)
        })
        .collect();
    Value::Object(counts)
}

fn entry_view(fields: &Extra) -> Result<Value, Box<HelperError>> {
    let mut view = Map::new();
    for name in DETAIL_KEYS {
        let value = fields.get(name).map_or(Ok(Value::Null), |value| {
            serde_json::to_value(value).or_value_error()
        })?;
        view.insert(name.to_owned(), value);
    }
    Ok(Value::Object(view))
}

fn logging_view(logging_obj: &StreamLoggingObject) -> Result<Value, Box<HelperError>> {
    let Some(logged) = logging_obj.read(&mut |state| Logged {
        model: state.model.clone(),
        provider: state.custom_llm_provider.clone(),
        call_type: state.call_type.clone(),
        details: state.model_call_details.clone(),
        optional: state.optional_params.clone(),
    }) else {
        return Err(helper_error(
            "LoggingState",
            "the logging object holds no state",
        ));
    };
    let mut view = Map::new();
    view.insert("model".to_owned(), json!(logged.model));
    view.insert("custom_llm_provider".to_owned(), json!(logged.provider));
    view.insert("call_type".to_owned(), json!(logged.call_type));
    view.insert("details".to_owned(), entry_view(&logged.details)?);
    view.insert(
        "optional".to_owned(),
        json!({ "vertex_location": optional_view(&logged.optional) }),
    );
    Ok(Value::Object(view))
}

fn optional_view(optional: &IndexMap<String, Json>) -> Value {
    optional
        .get("vertex_location")
        .map_or(Value::Null, |value| {
            serde_json::to_value(value).unwrap_or(Value::Null)
        })
}

struct Logged {
    model: String,
    provider: Option<String>,
    call_type: String,
    details: Extra,
    optional: Extra,
}

fn cost_view(cost: &RecordingCost) -> Value {
    let rendered: Vec<Value> = cost
        .calls
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .map(|call| {
            let usage = call
                .usage
                .map_or(Value::Null, |(prompt, completion, total)| {
                    json!([prompt, completion, total])
                });
            json!({
                "model": call.model,
                "custom_llm_provider": call.provider,
                "response_model": call.response_model,
                "usage": usage,
                "kind": call.kind,
                "call_type": call.call_type,
                "vertex_location": call.vertex_location,
            })
        })
        .collect();
    Value::Array(rendered)
}

pub(super) fn helper_error(class: &str, message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}
