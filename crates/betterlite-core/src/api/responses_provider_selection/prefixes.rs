use super::ParameterMap;
use betterlite_models::controls::json_value::JsonValue;
pub const OPENAI_CHAT_PREFIX: &str = "openai/chat_completions/";
pub const RESPONSES_ROUTING_PREFIX: &str = "responses/";
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
#[must_use]
pub fn strip_responses_routing_prefix(model: &str) -> String {
    model
        .strip_prefix(RESPONSES_ROUTING_PREFIX)
        .unwrap_or(model)
        .to_owned()
}
pub fn pop_use_chat_completions_api(kwargs: &mut ParameterMap) -> bool {
    match kwargs.shift_remove("use_chat_completions_api") {
        Some(JsonValue::Null) | None => false,
        Some(JsonValue::Bool(flag)) => flag,
        Some(JsonValue::Number(number)) => number.as_f64().is_some_and(|float| float.abs() > 0.0),
        Some(JsonValue::Text(text)) => !text.is_empty(),
        Some(JsonValue::Array(items)) => !items.is_empty(),
        Some(JsonValue::Object(fields)) => !fields.is_empty(),
    }
}
