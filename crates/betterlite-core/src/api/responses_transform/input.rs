use betterlite_models::controls::json_value::JsonValue;

use super::content::transform_message;
use super::error::ResponsesTransformError;
use super::roles::{is_history_item, responses_role};

pub(super) fn transform_input(input: &JsonValue) -> Result<JsonValue, ResponsesTransformError> {
    match input {
        JsonValue::Text(text) => Ok(JsonValue::Text(text.clone())),
        JsonValue::Array(items) => transform_list(items),
        JsonValue::Object(fields) => {
            let content: Vec<&JsonValue> = match fields.get("content") {
                Some(JsonValue::Array(items)) => items.iter().collect(),
                None
                | Some(
                    JsonValue::Null
                    | JsonValue::Bool(_)
                    | JsonValue::Number(_)
                    | JsonValue::Text(_)
                    | JsonValue::Object(_),
                ) => vec![input],
            };
            Ok(JsonValue::Array(vec![transform_message(&content, "user")?]))
        }
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) => {
            Err(ResponsesTransformError::NotInput { field: "input" })
        }
    }
}

fn transform_list(items: &[JsonValue]) -> Result<JsonValue, ResponsesTransformError> {
    let mut messages = Vec::with_capacity(items.len());
    let mut has_history_item = false;
    for item in items {
        if !is_history_item(item) {
            continue;
        }
        has_history_item = true;
        messages.push(transform_history_item(item)?);
    }
    if has_history_item {
        return Ok(JsonValue::Array(messages));
    }
    let content: Vec<&JsonValue> = items.iter().collect();
    Ok(JsonValue::Array(vec![transform_message(&content, "user")?]))
}

fn transform_history_item(item: &JsonValue) -> Result<JsonValue, ResponsesTransformError> {
    let JsonValue::Object(fields) = item else {
        return Err(ResponsesTransformError::NotObject {
            field: "input item",
        });
    };
    let role = responses_role(fields)?;
    let content: Vec<&JsonValue> = match fields.get("content") {
        Some(JsonValue::Array(items)) => items.iter().collect(),
        None | Some(JsonValue::Null) => Vec::new(),
        Some(value) => vec![value],
    };
    transform_message(&content, &role)
}
