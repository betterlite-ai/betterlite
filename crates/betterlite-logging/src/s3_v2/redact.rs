use std::{collections::BTreeSet, mem};

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
    #[serde(default, skip_serializing_if = "NullableField::is_missing")]
    messages: NullableField<MessagesField>,
    #[serde(default, skip_serializing_if = "NullableField::is_missing")]
    response: NullableField<Json>,
    #[serde(flatten)]
    extra: Extra,
}

#[derive(Deserialize, Serialize, strum::EnumIs)]
#[serde(untagged)]
enum NullableField<Value> {
    #[strum(disabled)]
    Null(()),
    #[strum(disabled)]
    Value(Value),
    Missing,
}

impl<Value> Default for NullableField<Value> {
    fn default() -> Self {
        Self::Missing
    }
}

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum MessagesField {
    List(Vec<MessageEntry>),
    Other(Json),
}

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum MessageEntry {
    Message(Message),
    Other(Json),
}

#[derive(Deserialize, Serialize)]
struct Message {
    #[serde(flatten)]
    extra: Extra,
    #[serde(default, skip_serializing_if = "NullableField::is_missing")]
    content: NullableField<ContentField>,
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
    #[serde(
        rename = "type",
        default,
        skip_serializing_if = "NullableField::is_missing"
    )]
    kind: NullableField<Json>,
    #[serde(default, skip_serializing_if = "NullableField::is_missing")]
    file: NullableField<Json>,
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
        let current = mem::take(&mut payload.messages);
        payload.messages = match current {
            NullableField::Value(MessagesField::List(messages)) => NullableField::Value(
                strip_messages(MessagesField::List(messages), &pattern, max_depth),
            ),
            NullableField::Value(MessagesField::Other(value)) => {
                NullableField::Value(MessagesField::Other(value))
            }
            NullableField::Null(value) => NullableField::Null(value),
            NullableField::Missing => NullableField::Missing,
        };
    }
    if prompts_only {
        payload.response = NullableField::Null(());
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
                "s3 logging: recurse depth must be at least 0, using 10"
            );
            DEFAULT_MAX_DEPTH
        }
        None if configured.is_none() => DEFAULT_MAX_DEPTH,
        None => {
            tracing::warn!(
                ?configured,
                "s3 logging: recurse depth is not an integer, using 10"
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
        NullableField::Missing | NullableField::Null(()) => {
            ContentField::Other(redact_json(&Json::Null, pattern, 0, max_depth))
        }
        NullableField::Value(ContentField::Other(value)) => {
            ContentField::Other(redact_json(&value, pattern, 0, max_depth))
        }
        NullableField::Value(ContentField::Parts(parts)) => ContentField::Parts(
            parts
                .into_iter()
                .filter(keep_part)
                .map(|part| redact_part(part, pattern, max_depth))
                .collect(),
        ),
    };
    Message {
        extra,
        content: NullableField::Value(content),
    }
}

fn keep_part(part: &ContentPart) -> bool {
    match part {
        ContentPart::Other(_) => true,
        ContentPart::Object(object) => {
            matches!(object.file, NullableField::Missing)
                && !matches!(&object.kind, NullableField::Value(Json::Text(text)) if text != "text")
        }
    }
}

fn redact_part(part: ContentPart, pattern: &Regex, max_depth: i64) -> ContentPart {
    match part {
        ContentPart::Other(value) => ContentPart::Other(redact_json(&value, pattern, 0, max_depth)),
        ContentPart::Object(object) => ContentPart::Object(ContentObject {
            kind: match object.kind {
                NullableField::Value(value) => {
                    NullableField::Value(redact_json(&value, pattern, 1, max_depth))
                }
                NullableField::Null(()) => {
                    NullableField::Value(redact_json(&Json::Null, pattern, 1, max_depth))
                }
                NullableField::Missing => NullableField::Missing,
            },
            file: object.file,
            extra: object
                .extra
                .into_iter()
                .map(|(key, value)| (key, redact_json(&value, pattern, 1, max_depth)))
                .collect(),
        }),
    }
}

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
