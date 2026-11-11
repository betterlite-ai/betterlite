use crate::arguments::json::Json;

// LiteLLM validates a list before iterating it; any other JSON value is no list at all.
#[must_use]
pub fn object_list(value: &Json) -> Option<&[Json]> {
    match value {
        Json::List(items) => Some(items),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => None,
    }
}
