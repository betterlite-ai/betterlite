use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;
use serde::de::DeserializeOwned;

// The Converse path carries decisions::Json, while the shared tool helpers take the isomorphic prompts value.
#[must_use]
pub const fn as_object(value: &Json) -> Option<&IndexMap<String, Json>> {
    match value {
        Json::Object(fields) => Some(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

#[must_use]
pub const fn as_list(value: &Json) -> Option<&Vec<Json>> {
    match value {
        Json::List(items) => Some(items),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => None,
    }
}

#[must_use]
pub const fn as_text(value: &Json) -> Option<&str> {
    match value {
        Json::Text(text) => Some(text.as_str()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
    }
}

// The two trees differ only in their types, and IndexMap keeps key order through serde, so one pass converts either way.
pub fn convert_all<Source, Target>(values: &[Source]) -> Result<Vec<Target>, serde_json::Error>
where
    Source: Serialize,
    Target: DeserializeOwned,
{
    values
        .iter()
        .map(|value| serde_json::from_str(&serde_json::to_string(value)?))
        .collect()
}
