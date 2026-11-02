use crate::arguments::json::{Json, Object};

// Matches LiteLLM's DEFAULT_MAX_RECURSE_DEPTH; a branch deeper than this is returned unfiltered.
pub const MAX_RECURSE_DEPTH: u32 = 100;

#[must_use]
pub fn filter_value_from_dict(dictionary: &Object, key: &str, depth: u32) -> Object {
    if depth > MAX_RECURSE_DEPTH {
        return dictionary.clone();
    }
    dictionary
        .iter()
        .filter(|(existing_key, _)| existing_key.as_str() != key)
        .map(|(existing_key, value)| {
            (
                existing_key.clone(),
                filter_value_at_depth(value, key, depth.saturating_add(1)),
            )
        })
        .collect()
}

// LiteLLM only recurses into dict items a list carries; other items pass through untouched.
fn filter_value_at_depth(value: &Json, key: &str, depth: u32) -> Json {
    match value {
        Json::Object(object) => Json::Object(filter_value_from_dict(object, key, depth)),
        Json::List(items) => Json::List(
            items
                .iter()
                .map(|item| match item {
                    Json::Object(object) => {
                        Json::Object(filter_value_from_dict(object, key, depth))
                    }
                    Json::Null
                    | Json::Bool(_)
                    | Json::Number(_)
                    | Json::Text(_)
                    | Json::List(_) => item.clone(),
                })
                .collect(),
        ),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => value.clone(),
    }
}

mod tests;
