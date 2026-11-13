use betterlite_models::decisions::Json;

use crate::fields::payload::StandardLoggingPayload;

pub const MAX_LOGGED_TEXT_CHARS: usize = 10_000;
pub const TRUNCATION_SUFFIX: &str =
    "...truncated by litellm, this logger does not support large content";

#[must_use]
// ast-grep-ignore: no-long-names
pub fn truncate_standard_logging_payload_content(
    payload: &StandardLoggingPayload,
) -> StandardLoggingPayload {
    let mut truncated = payload.clone();
    if let Some(text) = payload.error_str.as_deref().and_then(truncated_text) {
        truncated.error_str = Some(text);
    }
    if let Some(text) = payload
        .messages
        .as_ref()
        .map(field_text)
        .as_deref()
        .and_then(truncated_text)
    {
        truncated.messages = Some(Json::Text(text));
    }
    if let Some(text) = payload
        .response
        .as_ref()
        .map(field_text)
        .as_deref()
        .and_then(truncated_text)
    {
        truncated.response = Some(Json::Text(text));
    }
    truncated
}

fn field_text(value: &Json) -> String {
    match value {
        Json::Text(text) => text.clone(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            // a Json value always serializes; the fallback only satisfies the Result
            serde_json::to_string(value).unwrap_or_default()
        }
    }
}

fn truncated_text(text: &str) -> Option<String> {
    let mut head = String::new();
    let mut characters = text.chars();
    for _ in 0..MAX_LOGGED_TEXT_CHARS {
        head.push(characters.next()?);
    }
    characters.next()?;
    head.push_str(TRUNCATION_SUFFIX);
    Some(head)
}
