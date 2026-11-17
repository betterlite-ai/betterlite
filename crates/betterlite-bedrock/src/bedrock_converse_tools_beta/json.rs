use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::Json as PromptsJson;
use indexmap::IndexMap;

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

#[must_use]
pub fn to_prompts(value: &Json) -> PromptsJson {
    match value {
        Json::Null => PromptsJson::Null,
        Json::Bool(flag) => PromptsJson::Bool(*flag),
        Json::Number(number) => PromptsJson::Number(number.clone()),
        Json::Text(text) => PromptsJson::Text(text.clone()),
        Json::List(items) => PromptsJson::List(items.iter().map(to_prompts).collect()),
        Json::Object(fields) => PromptsJson::Object(
            fields
                .iter()
                .map(|(name, entry)| (name.clone(), to_prompts(entry)))
                .collect(),
        ),
    }
}

#[must_use]
pub fn from_prompts(value: &PromptsJson) -> Json {
    match value {
        PromptsJson::Null => Json::Null,
        PromptsJson::Bool(flag) => Json::Bool(*flag),
        PromptsJson::Number(number) => Json::Number(number.clone()),
        PromptsJson::Text(text) => Json::Text(text.clone()),
        PromptsJson::List(items) => Json::List(items.iter().map(from_prompts).collect()),
        PromptsJson::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(name, entry)| (name.clone(), from_prompts(entry)))
                .collect(),
        ),
    }
}
