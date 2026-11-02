use serde_json::{Map, Value};

use super::{CODE_EXECUTION_TOOL_NAME, CodeExecutionToolCall};

#[must_use]
pub fn output_item_type(item: &Value) -> Option<Value> {
    item.as_object()
        .and_then(|object| object.get("type"))
        .cloned()
}

#[must_use]
pub fn response_output(response: &Value) -> Value {
    response
        .as_object()
        .and_then(|object| object.get("output"))
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()))
}

#[must_use]
pub fn tool_call_arguments(arguments: Option<&Value>) -> String {
    match arguments {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => serde_json::to_string(other).unwrap_or_default(),
    }
}

#[must_use]
pub fn narrow_tool_call(tool_call: &Map<String, Value>) -> CodeExecutionToolCall {
    CodeExecutionToolCall {
        id: tool_call
            .get("id")
            .and_then(Value::as_str)
            .map(String::from),
        call_id: tool_call
            .get("call_id")
            .and_then(Value::as_str)
            .map(String::from),
        kind: Some("function".to_owned()),
        name: CODE_EXECUTION_TOOL_NAME.to_owned(),
        arguments: tool_call_arguments(tool_call.get("arguments")),
    }
}

#[must_use]
pub fn extract_session_id(kwargs: &Map<String, Value>) -> Option<String> {
    for meta_key in ["metadata", "litellm_metadata"] {
        let session_id = kwargs
            .get(meta_key)
            .and_then(Value::as_object)
            .and_then(|metadata| metadata.get("session_id"))
            .and_then(Value::as_str);
        if let Some(found) = session_id {
            return Some(found.to_owned());
        }
    }
    None
}

#[must_use]
pub fn extract_identity(kwargs: &Map<String, Value>) -> String {
    kwargs
        .get("user_api_key_hash")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

include!("helpers/tests/cases.rs");
