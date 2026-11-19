use betterlite_models::controls::json_value::JsonValue;

use super::ParameterMap;

pub const OPENAI_CHAT_PREFIX: &str = "openai/chat_completions/";
pub const RESPONSES_ROUTING_PREFIX: &str = "responses/";

// LiteLLM's _normalize_openai_chat_completions_responses_model: only a nonempty remainder is rewritten
#[must_use]
pub fn normalize_openai_chat_model(model: &str) -> (String, bool) {
    let Some(remainder) = model.strip_prefix(OPENAI_CHAT_PREFIX) else {
        return (model.to_owned(), false);
    };
    if remainder.is_empty() {
        return (model.to_owned(), false);
    }
    (format!("openai/{remainder}"), true)
}

// LiteLLM's _strip_responses_routing_prefix: the first responses/ segment names the surface, not the model
#[must_use]
pub fn strip_responses_routing_prefix(model: &str) -> String {
    model
        .strip_prefix(RESPONSES_ROUTING_PREFIX)
        .unwrap_or(model)
        .to_owned()
}

// LiteLLM's _pop_use_chat_completions_api_kw: the value is popped and read with Python truthiness
pub fn pop_use_chat_completions_api(kwargs: &mut ParameterMap) -> bool {
    match kwargs.shift_remove("use_chat_completions_api") {
        Some(value) => value_as_flag(&value),
        None => false,
    }
}

#[must_use]
pub fn value_as_flag(value: &JsonValue) -> bool {
    match value {
        JsonValue::Null => false,
        JsonValue::Bool(flag) => *flag,
        JsonValue::Number(number) => number.as_f64().is_some_and(|float| float.abs() > 0.0),
        JsonValue::Text(text) => !text.is_empty(),
        JsonValue::Array(items) => !items.is_empty(),
        JsonValue::Object(fields) => !fields.is_empty(),
    }
}
