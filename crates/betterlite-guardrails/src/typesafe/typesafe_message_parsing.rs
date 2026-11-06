use std::str;

use betterlite_prompts::arguments::json::{Json, Object};

#[must_use]
pub const fn as_str_object_dict(value: &Json) -> Option<&Object> {
    match value {
        Json::Object(fields) => Some(fields),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}

#[must_use]
pub fn as_object_list(value: &Json) -> Option<&[Json]> {
    match value {
        Json::List(items) => Some(items),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_) => None,
    }
}

// httpx reports a DecodingError for a body it cannot decode, and LiteLLM substitutes this marker before truncating.
#[must_use]
pub fn safe_response_text(body: Option<&[u8]>, limit: usize) -> String {
    let Some(bytes) = body else {
        return String::new();
    };
    let Ok(text) = str::from_utf8(bytes) else {
        return "<undecodable response body>".to_owned();
    };
    text.chars().take(limit).collect()
}

#[path = "typesafe_message_parsing/tests.rs"]
mod tests;
