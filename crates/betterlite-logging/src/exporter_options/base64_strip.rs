use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use regex::Regex;

use crate::fields::payload::StandardLoggingPayload;

pub const BASE64_REDACTED: &str = "[BASE64_REDACTED]";
pub const MAX_DEPTH_REACHED: &str = "[MAX_DEPTH_REACHED]";

const BASE64_PATTERN: &str =
    r"data:(?:application|image|audio|video)/[a-zA-Z0-9.+-]+;base64,[A-Za-z0-9+/=\s]+";

#[derive(Debug, thiserror::Error)]
pub enum Base64StripError {
    #[error("invalid base64 redaction pattern")]
    Pattern(#[from] regex::Error),
}

pub fn strip_base64_from_messages(
    payload: &StandardLoggingPayload,
    max_depth: i64,
) -> Result<StandardLoggingPayload, Base64StripError> {
    let Some(Json::List(messages)) = payload.messages.as_ref() else {
        return Ok(payload.clone());
    };
    let pattern = Regex::new(BASE64_PATTERN)?;
    let mut stripped = payload.clone();
    stripped.messages = Some(Json::List(process_messages(messages, &pattern, max_depth)));
    Ok(stripped)
}

fn process_messages(messages: &[Json], pattern: &Regex, max_depth: i64) -> Vec<Json> {
    messages
        .iter()
        .filter_map(|message| match message {
            Json::Object(fields) => Some(Json::Object(process_message(fields, pattern, max_depth))),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
        })
        .collect()
}

fn process_message(
    fields: &IndexMap<String, Json>,
    pattern: &Regex,
    max_depth: i64,
) -> IndexMap<String, Json> {
    let mut processed = IndexMap::with_capacity(fields.len().saturating_add(1));
    for (key, value) in fields {
        let redacted = if key == "content" {
            redact_content(value, pattern, max_depth)
        } else {
            redact_value(value, 0, max_depth, pattern)
        };
        processed.insert(key.clone(), redacted);
    }
    // LiteLLM assigns a content key even when the message had none, so a missing one reads back as null
    if !fields.contains_key("content") {
        processed.insert(
            "content".to_owned(),
            redact_value(&Json::Null, 0, max_depth, pattern),
        );
    }
    processed
}

fn redact_content(value: &Json, pattern: &Regex, max_depth: i64) -> Json {
    let Json::List(parts) = value else {
        return redact_value(value, 0, max_depth, pattern);
    };
    let kept = parts.iter().filter(|part| match part {
        Json::Object(fields) => {
            !fields.contains_key("file")
                && !matches!(fields.get("type"), Some(Json::Text(kind)) if kind != "text")
        }
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => true,
    });
    Json::List(
        kept.map(|part| redact_value(part, 0, max_depth, pattern))
            .collect(),
    )
}

fn redact_value(value: &Json, depth: i64, max_depth: i64, pattern: &Regex) -> Json {
    if depth > max_depth {
        tracing::warn!(
            max_depth,
            "base64 redaction reached the configured recursion depth"
        );
        return Json::Text(MAX_DEPTH_REACHED.to_owned());
    }
    match value {
        Json::Text(text) => Json::Text(pattern.replace_all(text, BASE64_REDACTED).into_owned()),
        Json::List(items) => Json::List(
            items
                .iter()
                .map(|item| redact_value(item, depth.saturating_add(1), max_depth, pattern))
                .collect(),
        ),
        Json::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(key, item)| {
                    (
                        key.clone(),
                        redact_value(item, depth.saturating_add(1), max_depth, pattern),
                    )
                })
                .collect(),
        ),
        Json::Null | Json::Bool(_) | Json::Number(_) => value.clone(),
    }
}
