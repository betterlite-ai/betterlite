use std::collections::BTreeSet;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::decisions::{Extra, Json};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::s3_transport::option_value::integer_from_text;
use crate::s3_v2::Feature;
use crate::s3_v2::events::EventError;

const MAX_DEPTH_VARIABLE: &str = "DEFAULT_MAX_RECURSE_DEPTH_SENSITIVE_DATA_MASKER";
const DEFAULT_MAX_DEPTH: i64 = 10;
const BASE64: &str =
    r"data:(?:application|image|audio|video)/[a-zA-Z0-9.+-]+;base64,[A-Za-z0-9+/=\s]+";

#[derive(Deserialize, Serialize)]
struct Payload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    messages: Option<MessagesField>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    response: Option<Json>,
    #[serde(flatten)]
    extra: Extra,
}

// LiteLLM reads payload["messages"] as a list and otherwise leaves it untouched
#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum MessagesField {
    List(Vec<MessageEntry>),
    Other(Json),
}

// a non-object entry is dropped by _process_messages' `if not isinstance(msg, dict): continue`
#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum MessageEntry {
    Message(Message),
    Other(Json),
}

#[derive(Deserialize, Serialize)]
struct Message {
    #[serde(default)]
    content: Option<ContentField>,
    #[serde(flatten)]
    extra: Extra,
}

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum ContentField {
    Parts(Vec<ContentPart>),
    Other(Json),
}

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum ContentPart {
    Object(ContentObject),
    Other(Json),
}

#[derive(Deserialize, Serialize)]
struct ContentObject {
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    kind: Option<Json>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    file: Option<Json>,
    #[serde(flatten)]
    extra: Extra,
}

pub(super) fn project(
    encoded: &str,
    features: &BTreeSet<Feature>,
    prompts_only: bool,
    environment: &dyn EnvironmentLookup,
) -> Result<String, EventError> {
    let mut payload: Payload = serde_json::from_str(encoded)?;
    if features.contains(&Feature::StripBase64) {
        let pattern = Regex::new(BASE64)?;
        let max_depth = resolve_max_depth(environment);
        payload.messages = payload
            .messages
            .map(|messages| strip_messages(messages, &pattern, max_depth));
    }
    if prompts_only {
        payload.response = Some(Json::Null);
    }
    Ok(serde_json::to_string(&payload)?)
}

fn resolve_max_depth(environment: &dyn EnvironmentLookup) -> i64 {
    let configured = environment.lookup(MAX_DEPTH_VARIABLE);
    match configured.as_deref().and_then(integer_from_text) {
        Some(depth) if depth >= 0 => depth,
        Some(_) => {
            tracing::warn!(
                ?configured,
                "s3 logging: DEFAULT_MAX_RECURSE_DEPTH_SENSITIVE_DATA_MASKER must be at least 0, using 10"
            );
            DEFAULT_MAX_DEPTH
        }
        None if configured.is_none() => DEFAULT_MAX_DEPTH,
        None => {
            tracing::warn!(
                ?configured,
                "s3 logging: DEFAULT_MAX_RECURSE_DEPTH_SENSITIVE_DATA_MASKER is not an integer, using 10"
            );
            DEFAULT_MAX_DEPTH
        }
    }
}

fn strip_messages(messages: MessagesField, pattern: &Regex, max_depth: i64) -> MessagesField {
    let MessagesField::List(entries) = messages else {
        return messages;
    };
    let processed = entries
        .into_iter()
        .filter_map(|entry| match entry {
            MessageEntry::Other(_) => None,
            MessageEntry::Message(message) => Some(MessageEntry::Message(strip_message(
                message, pattern, max_depth,
            ))),
        })
        .collect();
    MessagesField::List(processed)
}

fn strip_message(message: Message, pattern: &Regex, max_depth: i64) -> Message {
    let extra = message
        .extra
        .into_iter()
        .map(|(key, value)| (key, redact_json(&value, pattern, 0, max_depth)))
        .collect();
    let content = match message.content {
        None => ContentField::Other(redact_json(&Json::Null, pattern, 0, max_depth)),
        Some(ContentField::Other(value)) => {
            ContentField::Other(redact_json(&value, pattern, 0, max_depth))
        }
        Some(ContentField::Parts(parts)) => ContentField::Parts(
            parts
                .into_iter()
                .filter(keep_part)
                .map(|part| redact_part(part, pattern, max_depth))
                .collect(),
        ),
    };
    Message {
        content: Some(content),
        extra,
    }
}

// mirrors CustomLogger._should_keep_content: drop a "file" entry or a typed, non-text entry
fn keep_part(part: &ContentPart) -> bool {
    match part {
        ContentPart::Other(_) => true,
        ContentPart::Object(object) => {
            object.file.is_none()
                && !matches!(&object.kind, Some(Json::Text(text)) if text != "text")
        }
    }
}

fn redact_part(part: ContentPart, pattern: &Regex, max_depth: i64) -> ContentPart {
    match part {
        ContentPart::Other(value) => ContentPart::Other(redact_json(&value, pattern, 0, max_depth)),
        ContentPart::Object(object) => ContentPart::Object(ContentObject {
            kind: object
                .kind
                .map(|value| redact_json(&value, pattern, 1, max_depth)),
            file: object.file,
            extra: object
                .extra
                .into_iter()
                .map(|(key, value)| (key, redact_json(&value, pattern, 1, max_depth)))
                .collect(),
        }),
    }
}

// mirrors CustomLogger._redact_base64: each object or list level adds one to the recursion depth
fn redact_json(value: &Json, pattern: &Regex, depth: i64, max_depth: i64) -> Json {
    if depth > max_depth {
        return Json::Text("[MAX_DEPTH_REACHED]".to_owned());
    }
    match value {
        Json::Text(text) => Json::Text(pattern.replace_all(text, "[BASE64_REDACTED]").into_owned()),
        Json::List(items) => Json::List(
            items
                .iter()
                .map(|item| redact_json(item, pattern, depth.saturating_add(1), max_depth))
                .collect(),
        ),
        Json::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(key, item)| {
                    (
                        key.clone(),
                        redact_json(item, pattern, depth.saturating_add(1), max_depth),
                    )
                })
                .collect(),
        ),
        Json::Null | Json::Bool(_) | Json::Number(_) => value.clone(),
    }
}
