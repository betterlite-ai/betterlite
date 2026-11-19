use betterlite_models::decisions::Json;

// LiteLLM's truthiness on a configured value: null, false, zero, and empty text, list or map are false.
#[must_use]
pub fn holds(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(flag) => *flag,
        Json::Number(number) => number
            .as_f64()
            .is_none_or(|amount| amount > 0.0 || amount < 0.0),
        Json::Text(text) => !text.is_empty(),
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}
