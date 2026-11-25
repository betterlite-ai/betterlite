use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_pyhost::json_values::{JsonObject, JsonValue};

// LiteLLM calls the handler with **arguments, so every object level becomes one Python keyword mapping
pub(super) fn keyword_arguments(arguments: &Object) -> JsonObject {
    arguments
        .iter()
        .map(|(name, value)| (name.clone(), json_value(value)))
        .collect()
}

fn json_value(value: &Json) -> JsonValue {
    match value {
        Json::Null => JsonValue::Null,
        Json::Bool(flag) => JsonValue::Bool(*flag),
        Json::Number(number) => JsonValue::Number(number.clone()),
        Json::Text(text) => JsonValue::String(text.clone()),
        Json::List(items) => JsonValue::Array(items.iter().map(json_value).collect()),
        Json::Object(fields) => JsonValue::Object(keyword_arguments(fields)),
    }
}
