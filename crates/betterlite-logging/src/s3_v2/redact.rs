use std::collections::BTreeMap;

use regex::Regex;
use serde_json::value::{RawValue, to_raw_value};

use crate::s3_v2::Feature;
use crate::s3_v2::events::EventError;
use std::collections::BTreeSet;

// Provider-specific message fields pass through unchanged except for base64 redaction.
type Fields = BTreeMap<String, Box<RawValue>>;
const MAX_DEPTH: u8 = 10;
const BASE64: &str =
    r"data:(?:application|image|audio|video)/[a-zA-Z0-9.+-]+;base64,[A-Za-z0-9+/=\s]+";

pub(super) fn project(
    encoded: &str,
    features: &BTreeSet<Feature>,
    prompts_only: bool,
) -> Result<String, EventError> {
    let mut payload: Fields = serde_json::from_str(encoded)?;
    if features.contains(&Feature::StripBase64)
        && let Some(messages) = payload.get_mut("messages")
    {
        let pattern = Regex::new(BASE64)?;
        *messages = strip_messages(messages, &pattern)?;
    }
    if prompts_only {
        payload.insert("response".to_owned(), to_raw_value(&())?);
    }
    Ok(serde_json::to_string(&payload)?)
}

fn strip_messages(raw: &RawValue, pattern: &Regex) -> Result<Box<RawValue>, serde_json::Error> {
    if !raw.get().starts_with('[') {
        return to_raw_value(raw);
    }
    let messages: Vec<Box<RawValue>> = serde_json::from_str(raw.get())?;
    let mut output = Vec::new();
    for message in messages {
        if !message.get().starts_with('{') {
            continue;
        }
        let mut fields: Fields = serde_json::from_str(message.get())?;
        let raw_content = fields.remove("content").unwrap_or(to_raw_value(&())?);
        let content = if raw_content.get().starts_with('[') {
            let parts = serde_json::from_str(raw_content.get())?;
            to_raw_value(&strip_parts(parts, pattern)?)?
        } else {
            redact(&raw_content, pattern, 0)?
        };
        for value in fields.values_mut() {
            *value = redact(value, pattern, 0)?;
        }
        fields.insert("content".to_owned(), content);
        output.push(fields);
    }
    to_raw_value(&output)
}

fn keep_part(part: &RawValue) -> Result<bool, serde_json::Error> {
    if !part.get().starts_with('{') {
        return Ok(true);
    }
    let fields: Fields = serde_json::from_str(part.get())?;
    let kind = fields
        .get("type")
        .and_then(|raw| serde_json::from_str::<String>(raw.get()).ok());
    Ok(!fields.contains_key("file") && kind.as_deref().is_none_or(|name| name == "text"))
}

fn redact(raw: &RawValue, pattern: &Regex, depth: u8) -> Result<Box<RawValue>, serde_json::Error> {
    if depth > MAX_DEPTH {
        return to_raw_value("[MAX_DEPTH_REACHED]");
    }
    match raw.get().as_bytes().first() {
        Some(b'"') => {
            let text: String = serde_json::from_str(raw.get())?;
            to_raw_value(&pattern.replace_all(&text, "[BASE64_REDACTED]"))
        }
        Some(b'[') => {
            let mut items: Vec<Box<RawValue>> = serde_json::from_str(raw.get())?;
            for item in &mut items {
                *item = redact(item, pattern, depth.saturating_add(1))?;
            }
            to_raw_value(&items)
        }
        Some(b'{') => {
            let mut fields: Fields = serde_json::from_str(raw.get())?;
            for item in fields.values_mut() {
                *item = redact(item, pattern, depth.saturating_add(1))?;
            }
            to_raw_value(&fields)
        }
        _ => to_raw_value(raw),
    }
}

fn strip_parts(
    parts: Vec<Box<RawValue>>,
    pattern: &Regex,
) -> Result<Vec<Box<RawValue>>, serde_json::Error> {
    let mut retained = Vec::new();
    for part in parts {
        if keep_part(&part)? {
            retained.push(redact(&part, pattern, 0)?);
        }
    }
    Ok(retained)
}
