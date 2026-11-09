use std::sync::{Arc, Mutex, PoisonError};

use betterlite_logging::rubrik::request_data::{ResponseSource, TrackedField};
use betterlite_logging::rubrik::tool_calls::ToolCallSource;
use betterlite_logging::rubrik_moderation::context::{
    LoggingObject, LoggingObjectStash, RequestData,
};
use betterlite_logging::rubrik_moderation::error::ModifyResponseException;
use betterlite_logging::rubrik_moderation::inputs::GuardrailInputs;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::{Value, json};

use crate::helpers::HelperError;

pub(super) fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "RubrikModerationError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn optional_text(value: &Value, name: &str) -> Result<Option<String>, Box<HelperError>> {
    match value {
        Value::Null => Ok(None),
        Value::String(text) => Ok(Some(text.clone())),
        Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_) => {
            Err(failure(format!("{name} must be a string or null")))
        }
    }
}

fn json_value(value: &Value) -> Result<Json, Box<HelperError>> {
    serde_json::from_value(value.clone())
        .map_err(|error| failure(format!("the value is not JSON: {error}")))
}

fn json_of(value: &Json) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

fn response_source(value: &Value) -> Result<ResponseSource, Box<HelperError>> {
    match value {
        Value::Null => Ok(ResponseSource::Absent),
        Value::Object(fields) => match fields.get("model") {
            Some(model) => Ok(ResponseSource::WithModel(optional_text(
                model,
                "request_data.response.model",
            )?)),
            None => Ok(ResponseSource::WithoutModel),
        },
        Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => Err(failure(
            "request_data.response must be an object or null".to_owned(),
        )),
    }
}

pub(super) fn request_data(input: Option<&Value>) -> Result<RequestData, Box<HelperError>> {
    let Some(Value::Object(fields)) = input else {
        return match input {
            None | Some(Value::Null) => Ok(RequestData::default()),
            Some(_) => Err(failure("request_data must be an object".to_owned())),
        };
    };
    let mut data = RequestData::default();
    for (name, entry) in fields {
        match name.as_str() {
            "litellm_call_id" => data.litellm_call_id = optional_text(entry, name)?,
            "model" => data.model = optional_text(entry, name)?,
            "user" => data.user = optional_text(entry, name)?,
            "correlation_key" => data.correlation_key = optional_text(entry, name)?,
            "tools" => data.tools = TrackedField::Present(json_value(entry)?),
            "response" => data.response = response_source(entry)?,
            other => {
                return Err(failure(format!("unknown request_data field {other}")));
            }
        }
    }
    Ok(data)
}

pub(super) fn logging_object(
    input: Option<&Value>,
) -> Result<Option<Arc<LoggingObject>>, Box<HelperError>> {
    let Some(value) = input else {
        return Ok(None);
    };
    let Value::Object(fields) = value else {
        return match value {
            Value::Null => Ok(None),
            Value::Bool(_)
            | Value::Number(_)
            | Value::String(_)
            | Value::Array(_)
            | Value::Object(_) => Err(failure("logging_obj must be an object or null".to_owned())),
        };
    };
    let mut details = IndexMap::new();
    for (name, entry) in fields {
        match name.as_str() {
            "model_call_details" => match entry {
                Value::Null => {}
                Value::Object(entries) => {
                    for (entry_name, nested) in entries {
                        details.insert(entry_name.clone(), json_value(nested)?);
                    }
                }
                Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => {
                    return Err(failure(
                        "logging_obj.model_call_details must be an object".to_owned(),
                    ));
                }
            },
            other => {
                return Err(failure(format!("unknown logging_obj field {other}")));
            }
        }
    }
    Ok(Some(Arc::new(LoggingObject {
        model_call_details: Mutex::new(details),
    })))
}

pub(super) fn request_data_view(data: &RequestData) -> Value {
    let tools = match &data.tools {
        TrackedField::Present(tools) => json_of(tools),
        TrackedField::Missing => Value::Null,
    };
    let stash = match &data.logging_obj {
        LoggingObjectStash::Missing => "missing",
        LoggingObjectStash::Present(None) => "null",
        LoggingObjectStash::Present(Some(_)) => "same",
    };
    let response_model = match &data.response {
        ResponseSource::WithModel(Some(model)) => json!(model),
        ResponseSource::Absent | ResponseSource::WithoutModel | ResponseSource::WithModel(None) => {
            Value::Null
        }
    };
    json!({
        "litellm_call_id": &data.litellm_call_id,
        "model": &data.model,
        "user": &data.user,
        "correlation_key": &data.correlation_key,
        "tools_present": matches!(data.tools, TrackedField::Present(_)),
        "tools": tools,
        "response_present": !matches!(data.response, ResponseSource::Absent),
        "response_has_model": matches!(data.response, ResponseSource::WithModel(_)),
        "response_model": response_model,
        "stash": stash,
    })
}

fn tool_call_view(source: &ToolCallSource) -> Value {
    match source {
        ToolCallSource::Raw(raw) => json_of(raw),
        ToolCallSource::Normalized(call) => serde_json::to_value(call).unwrap_or(Value::Null),
        ToolCallSource::Protocol { id, kind, function } => json!({
            "id": id,
            "type": kind.clone().unwrap_or_else(|| "function".to_owned()),
            "function": serde_json::to_value(function).unwrap_or(Value::Null),
        }),
    }
}

pub(super) fn inputs_view(inputs: &GuardrailInputs) -> Value {
    json!({
        "model": &inputs.model,
        "texts": &inputs.texts,
        "tool_calls": inputs.tool_calls.iter().map(tool_call_view).collect::<Vec<Value>>(),
        "structured_messages": inputs.structured_messages.iter().map(json_of).collect::<Vec<Value>>(),
        "tools": inputs.tools.as_ref().map(json_of),
    })
}

pub(super) fn model_call_details_view(logging_obj: Option<&Arc<LoggingObject>>) -> Value {
    let Some(present) = logging_obj else {
        return Value::Null;
    };
    let details = present
        .model_call_details
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    serde_json::to_value(&*details).unwrap_or(Value::Null)
}

pub(super) fn block_view(exception: &ModifyResponseException) -> Value {
    json!({
        "message": exception.message,
        "model": exception.model,
        "guardrail_name": exception.guardrail_name,
        "request_data": request_data_view(&exception.request_data),
    })
}

pub(super) fn returned_result(
    inputs: &GuardrailInputs,
    request_data: &RequestData,
    logging_obj: Option<&Arc<LoggingObject>>,
) -> Value {
    json!({
        "outcome": "returned",
        "inputs": inputs_view(inputs),
        "block": Value::Null,
        "request_data": request_data_view(request_data),
        "model_call_details": model_call_details_view(logging_obj),
    })
}

pub(super) fn blocked_result(
    exception: &ModifyResponseException,
    request_data: &RequestData,
    logging_obj: Option<&Arc<LoggingObject>>,
) -> Value {
    json!({
        "outcome": "blocked",
        "inputs": Value::Null,
        "block": block_view(exception),
        "request_data": request_data_view(request_data),
        "model_call_details": model_call_details_view(logging_obj),
    })
}

pub(super) fn stashed_result(
    request_data: &RequestData,
    logging_obj: Option<&Arc<LoggingObject>>,
) -> Value {
    json!({
        "outcome": "stashed",
        "inputs": Value::Null,
        "block": Value::Null,
        "request_data": request_data_view(request_data),
        "model_call_details": model_call_details_view(logging_obj),
    })
}
