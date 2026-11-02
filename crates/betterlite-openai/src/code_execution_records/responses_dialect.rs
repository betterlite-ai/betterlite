use serde_json::{Map, Value};

use super::helpers::{output_item_type, response_output, tool_call_arguments};
use super::{CODE_EXECUTION_TOOL_NAME, CodeExecutionToolCall};

#[must_use]
pub fn normalize_messages(messages: &Value) -> Vec<Value> {
    match messages {
        Value::String(text) => vec![Value::Object(Map::from_iter([
            ("role".to_owned(), Value::String("user".to_owned())),
            ("content".to_owned(), Value::String(text.clone())),
        ]))],
        Value::Array(items) => items.clone(),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Object(_) => Vec::new(),
    }
}

#[must_use]
pub fn is_code_execution_call(item: &Value) -> bool {
    let Some(object) = item.as_object() else {
        return false;
    };
    object.get("type").and_then(Value::as_str) == Some("function_call")
        && object.get("name").and_then(Value::as_str) == Some(CODE_EXECUTION_TOOL_NAME)
}

#[must_use]
pub fn extract_tool_calls(response: &Value) -> Vec<CodeExecutionToolCall> {
    let Value::Array(output) = response_output(response) else {
        return Vec::new();
    };
    output
        .iter()
        .filter(|item| is_code_execution_call(item))
        .map(|item| CodeExecutionToolCall {
            id: None,
            call_id: item
                .as_object()
                .and_then(|object| object.get("call_id"))
                .and_then(Value::as_str)
                .map(String::from),
            kind: None,
            name: CODE_EXECUTION_TOOL_NAME.to_owned(),
            arguments: tool_call_arguments(
                item.as_object().and_then(|object| object.get("arguments")),
            ),
        })
        .collect()
}

#[must_use]
pub fn merge_calls(output: &Value, calls: &[Value]) -> Option<Vec<Value>> {
    let items = output.as_array()?;
    let insert_at = items
        .iter()
        .position(|item| output_item_type(item).as_ref().and_then(Value::as_str) == Some("message"))
        .unwrap_or(items.len());
    let (before, after) = items.split_at(insert_at);
    let mut merged = before.to_vec();
    merged.extend(calls.iter().cloned());
    merged.extend(after.iter().cloned());
    Some(merged)
}

include!("responses_dialect/tests/cases.rs");
