use betterlite_openai::chat::ResponseFormat;
use serde_json::Value;

use crate::schema::SchemaLimits;
use crate::schema::normalize::normalize_json_schema_types;

use super::GenAiInputError;
use super::input::GenerationConfig;

const JSON_MIME_TYPE: &str = "application/json";

pub(super) fn response_format(
    config: Option<&GenerationConfig>,
    limits: &SchemaLimits,
) -> Result<Option<ResponseFormat>, GenAiInputError> {
    let Some(fields) = config else {
        return Ok(None);
    };
    let mime = fields
        .response_mime_type
        .as_ref()
        .or(fields.response_mime_type_snake.as_ref());
    if mime.is_some_and(|given| given != JSON_MIME_TYPE) {
        return Ok(None);
    }
    let schema = fields
        .response_json_schema
        .as_ref()
        .or(fields.response_json_schema_snake.as_ref())
        .or(fields.response_schema.as_ref())
        .or(fields.response_schema_snake.as_ref());
    let Some(given) = schema.filter(|value| value.is_object()) else {
        return Ok(None);
    };
    let mut cleaned = normalize_json_schema_types(given, limits)?;
    if cleaned.get("type").and_then(Value::as_str) != Some("object") {
        return Ok(None);
    }
    without_gemini_keys(&mut cleaned, 0);
    let json_schema = serde_json::Map::from_iter([
        (
            String::from("name"),
            Value::String(String::from("response")),
        ),
        (String::from("schema"), cleaned),
    ]);
    Ok(Some(ResponseFormat {
        kind: String::from("json_schema"),
        json_schema: Some(Value::Object(json_schema)),
    }))
}

// LiteLLM's filter_value_from_dict visits dict values and the dicts a list carries at one level below the dict
fn without_gemini_keys(schema: &mut Value, depth: u32) {
    if depth > 100 {
        return;
    }
    let Some(fields) = schema.as_object_mut() else {
        return;
    };
    fields.retain(|key, _| !["propertyOrdering", "property_ordering"].contains(&key.as_str()));
    for value in fields.values_mut() {
        match value {
            Value::Object(_) => without_gemini_keys(value, depth.saturating_add(1)),
            Value::Array(items) => {
                for item in items.iter_mut().filter(|item| item.is_object()) {
                    without_gemini_keys(item, depth.saturating_add(1));
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }
}
