use betterlite_models::decisions::Json;

// LiteLLM's _optional_str; only a textual or numeric value has a meaningful string form, and Python's str() of the rest is not part of parity
#[must_use]
pub fn optional_str(value: Option<&Json>) -> Option<String> {
    match value? {
        Json::Text(text) => Some(text.clone()),
        Json::Number(number) => Some(number.to_string()),
        Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_) => None,
    }
}
