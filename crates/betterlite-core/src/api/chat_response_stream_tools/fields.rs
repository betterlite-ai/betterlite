use betterlite_logging::python_json::dumps;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::responses_tool_namespaces::values::has_value;

#[must_use]
pub const fn object(value: &Json) -> Option<&IndexMap<String, Json>> {
    if let Json::Object(fields) = value {
        return Some(fields);
    }
    None
}

#[must_use]
pub fn list(value: &Json) -> Option<&[Json]> {
    if let Json::List(items) = value {
        return Some(items);
    }
    None
}

// str(value) for the fields LiteLLM stringifies; containers keep JSON text, since Python's repr is not part of parity
#[must_use]
pub fn string_value(value: &Json) -> String {
    match value {
        Json::Null => "None".to_owned(),
        Json::Bool(true) => "True".to_owned(),
        Json::Bool(false) => "False".to_owned(),
        Json::Text(text) => text.clone(),
        Json::Number(_) | Json::List(_) | Json::Object(_) => dumps(value),
    }
}

#[must_use]
pub fn text_or_empty(value: Option<&Json>) -> String {
    value
        .filter(|candidate| has_value(candidate))
        .map_or_else(String::new, string_value)
}
