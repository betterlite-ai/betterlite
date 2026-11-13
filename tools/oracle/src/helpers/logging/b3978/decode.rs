use std::sync::{Arc, Mutex, PoisonError};

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

fn json_value(value: &Value) -> Result<Json, Box<HelperError>> {
    serde_json::from_value(value.clone())
        .map_err(|error| failure(format!("the value is not JSON: {error}")))
}

pub(super) fn request_data(input: Option<&Value>) -> Result<RequestData, Box<HelperError>> {
    let fields = match input {
        None | Some(Value::Null) => IndexMap::new(),
        Some(Value::Object(entries)) => {
            let mut fields = IndexMap::with_capacity(entries.len());
            for (name, entry) in entries {
                fields.insert(name.clone(), json_value(entry)?);
            }
            fields
        }
        Some(_) => return Err(failure("request_data must be an object".to_owned())),
    };
    Ok(RequestData {
        fields,
        logging_obj: LoggingObjectStash::Missing,
    })
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

fn tool_call_view(source: &ToolCallSource) -> Value {
    match source {
        ToolCallSource::Raw(raw) => serde_json::to_value(raw).unwrap_or(Value::Null),
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
        "structured_messages": inputs
            .structured_messages
            .iter()
            .map(|message| serde_json::to_value(message).unwrap_or(Value::Null))
            .collect::<Vec<Value>>(),
        "tools": inputs
            .tools
            .as_ref()
            .map(|tools| serde_json::to_value(tools).unwrap_or(Value::Null)),
    })
}

fn stash_status(data: &RequestData, logging_obj: Option<&Arc<LoggingObject>>) -> &'static str {
    match &data.logging_obj {
        LoggingObjectStash::Missing => "missing",
        LoggingObjectStash::Present(None) => "null",
        LoggingObjectStash::Present(Some(stashed)) => {
            if logging_obj.is_some_and(|present| Arc::ptr_eq(stashed, present)) {
                "same"
            } else {
                "other"
            }
        }
    }
}

fn blocked_flag(logging_obj: Option<&Arc<LoggingObject>>) -> bool {
    let Some(present) = logging_obj else {
        return false;
    };
    let details = present
        .model_call_details
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    matches!(details.get("_rubrik_blocked"), Some(Json::Bool(true)))
}

pub(super) fn outcome_result(
    outcome: Result<GuardrailInputs, ModifyResponseException>,
    request_data: &RequestData,
    logging_obj: Option<&Arc<LoggingObject>>,
) -> Value {
    let (outcome_label, inputs, block) = match outcome {
        Ok(returned) => ("returned", Some(inputs_view(&returned)), None),
        Err(exception) => (
            "blocked",
            None,
            Some(json!({
                "message": exception.message,
                "model": exception.model,
                "guardrail_name": exception.guardrail_name,
            })),
        ),
    };
    json!({
        "outcome": outcome_label,
        "inputs": inputs,
        "block": block,
        "stash": stash_status(request_data, logging_obj),
        "blocked": blocked_flag(logging_obj),
    })
}
