use std::collections::BTreeSet;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use regex::Regex;
use serde_json::value::RawValue;

use crate::s3_transport::option_value::integer_from_text;
use crate::s3_v2::Feature;
use crate::s3_v2::events::EventError;

const MAX_DEPTH_VARIABLE: &str = "DEFAULT_MAX_RECURSE_DEPTH_SENSITIVE_DATA_MASKER";
const DEFAULT_MAX_DEPTH: i64 = 10;
const BASE64: &str =
    r"data:(?:application|image|audio|video)/[a-zA-Z0-9.+-]+;base64,[A-Za-z0-9+/=\s]+";

type RawFields = IndexMap<String, Box<RawValue>>;

pub(super) fn project(
    encoded: &str,
    features: &BTreeSet<Feature>,
    prompts_only: bool,
    environment: &dyn EnvironmentLookup,
) -> Result<String, EventError> {
    let mut fields: RawFields = serde_json::from_str(encoded)?;
    if features.contains(&Feature::StripBase64) {
        let pattern = Regex::new(BASE64)?;
        let max_depth = resolve_max_depth(environment);
        if let Some(messages) = fields.get("messages").cloned()
            && let Some(processed) = strip_messages(&messages, &pattern, max_depth)?
        {
            fields.insert("messages".to_owned(), processed);
        }
    }
    if prompts_only {
        fields.insert("response".to_owned(), RawValue::NULL.to_owned());
    }
    Ok(serde_json::to_string(&fields)?)
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

// mirrors _strip_base64_from_messages_sync: a non-list or empty "messages" value stays untouched
fn strip_messages(
    raw: &RawValue,
    pattern: &Regex,
    max_depth: i64,
) -> Result<Option<Box<RawValue>>, EventError> {
    let Ok(items) = serde_json::from_str::<Vec<Box<RawValue>>>(raw.get()) else {
        return Ok(None);
    };
    if items.is_empty() {
        return Ok(None);
    }
    let mut kept = Vec::with_capacity(items.len());
    for item in items {
        // mirrors _process_messages' `if not isinstance(msg, dict): continue`
        let Ok(message) = serde_json::from_str::<RawFields>(item.get()) else {
            continue;
        };
        let processed = process_message(message, pattern, max_depth)?;
        kept.push(RawValue::from_string(serde_json::to_string(&processed)?)?);
    }
    Ok(Some(RawValue::from_string(serde_json::to_string(&kept)?)?))
}

// mirrors _process_messages: "content" is always reassigned, landing at the end when it was absent
fn process_message(
    mut message: RawFields,
    pattern: &Regex,
    max_depth: i64,
) -> Result<RawFields, EventError> {
    let content = message.get("content").cloned();
    let redacted_content = match content {
        Some(raw) => match serde_json::from_str::<Vec<Box<RawValue>>>(raw.get()) {
            Ok(items) => {
                let mut kept = Vec::with_capacity(items.len());
                for item in items {
                    if should_keep_content(&item) {
                        kept.push(redact_raw(&item, pattern, 0, max_depth)?);
                    }
                }
                RawValue::from_string(serde_json::to_string(&kept)?)?
            }
            Err(_) => redact_raw(&raw, pattern, 0, max_depth)?,
        },
        None => redact_raw(RawValue::NULL, pattern, 0, max_depth)?,
    };
    message.insert("content".to_owned(), redacted_content);
    for (key, value) in &mut message {
        if key != "content" {
            *value = redact_raw(value, pattern, 0, max_depth)?;
        }
    }
    Ok(message)
}

// mirrors _should_keep_content: drop an entry carrying a "file" key regardless of its value
fn should_keep_content(raw: &RawValue) -> bool {
    let Ok(content) = serde_json::from_str::<RawFields>(raw.get()) else {
        return true;
    };
    if content.contains_key("file") {
        return false;
    }
    content.get("type").is_none_or(|kind| {
        !serde_json::from_str::<String>(kind.get()).is_ok_and(|text| text != "text")
    })
}

fn redact_raw(
    raw: &RawValue,
    pattern: &Regex,
    depth: i64,
    max_depth: i64,
) -> Result<Box<RawValue>, EventError> {
    let value: Json = serde_json::from_str(raw.get())?;
    let redacted = redact_json(&value, pattern, depth, max_depth);
    Ok(RawValue::from_string(serde_json::to_string(&redacted)?)?)
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
