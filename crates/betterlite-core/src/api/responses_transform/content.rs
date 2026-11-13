use betterlite_models::controls::json_value::JsonValue;
use indexmap::IndexMap;

use super::error::ResponsesTransformError;

pub(super) fn transform_message(
    content: &[&JsonValue],
    role: &str,
) -> Result<JsonValue, ResponsesTransformError> {
    let mut items = Vec::with_capacity(content.len());
    for item in content {
        items.push(transform_content_item(*item, role)?);
    }
    let mut message = IndexMap::with_capacity(2);
    message.insert(String::from("role"), JsonValue::Text(role.to_owned()));
    message.insert(String::from("content"), JsonValue::Array(items));
    Ok(JsonValue::Object(message))
}

fn transform_content_item(
    item: &JsonValue,
    role: &str,
) -> Result<JsonValue, ResponsesTransformError> {
    let content_type = if role == "assistant" {
        "output_text"
    } else {
        "input_text"
    };
    match item {
        JsonValue::Text(text) => Ok(JsonValue::Object(text_content(content_type, text))),
        JsonValue::Object(fields) => transform_object_content(fields, content_type),
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Array(_) => {
            Err(ResponsesTransformError::NotContentItem)
        }
    }
}

fn transform_object_content(
    fields: &IndexMap<String, JsonValue>,
    content_type: &str,
) -> Result<JsonValue, ResponsesTransformError> {
    if !matches!(fields.get("type"), Some(JsonValue::Text(kind)) if kind.as_str() == "text") {
        return Ok(JsonValue::Object(fields.clone()));
    }
    let text = match fields.get("text") {
        None | Some(JsonValue::Null) => String::new(),
        Some(JsonValue::Text(text)) => text.clone(),
        Some(
            JsonValue::Bool(_) | JsonValue::Number(_) | JsonValue::Array(_) | JsonValue::Object(_),
        ) => {
            return Err(ResponsesTransformError::NotText {
                field: "content text",
            });
        }
    };
    Ok(JsonValue::Object(text_content(content_type, &text)))
}

fn text_content(content_type: &str, text: &str) -> IndexMap<String, JsonValue> {
    let mut content = IndexMap::with_capacity(2);
    content.insert(
        String::from("type"),
        JsonValue::Text(content_type.to_owned()),
    );
    content.insert(String::from("text"), JsonValue::Text(text.to_owned()));
    content
}
