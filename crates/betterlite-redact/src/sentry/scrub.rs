//! Walks a JSON event tree, scrubbing every string value.
//!
//! Uses the patterns from [`super::pattern`], except inside a few literal
//! source-context keys on the known stack-frame shapes that `sentry_scrubbing.py`
//! deliberately leaves untouched.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Number;

use super::pattern::{self, PatternError, Patterns};

pub(super) const MAX_SCRUB_DEPTH: usize = 64;
const SOURCE_CONTEXT_KEYS: &[&str] = &["pre_context", "context_line", "post_context"];
const STACK_FRAME_PATHS: &[&[&str]] = &[
    &["exception", "values", "*", "stacktrace", "frames", "*"],
    &["threads", "values", "*", "stacktrace", "frames", "*"],
    &["stacktrace", "frames", "*"],
];

/// A typed stand-in for the arbitrary JSON a Sentry event carries, mirroring the
/// shapes `scrub_json_strings` walks: strings, nested objects and arrays, and the
/// scalars it passes through untouched.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EventValue {
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

/// Scrubs every secret/PII repr pattern in `text`, mirroring `build_string_scrubber`.
pub fn scrub_text(text: &str, send_default_pii: bool) -> Result<String, PatternError> {
    pattern::scrub(text, send_default_pii)
}

/// Scrubs every string reachable from `event`, mirroring `scrub_json_strings`.
pub fn scrub_event(event: &EventValue, send_default_pii: bool) -> Result<EventValue, PatternError> {
    let patterns = pattern::build(send_default_pii)?;
    Ok(walk(event, &patterns, &[]))
}

fn is_stack_frame_path(path: &[String]) -> bool {
    STACK_FRAME_PATHS.iter().any(|candidate| {
        candidate.len() == path.len()
            && candidate
                .iter()
                .zip(path)
                .all(|(expected, actual)| *expected == actual.as_str())
    })
}

fn walk(value: &EventValue, patterns: &Patterns, path: &[String]) -> EventValue {
    if path.len() > MAX_SCRUB_DEPTH {
        return EventValue::String(pattern::FILTERED.to_owned());
    }
    match value {
        EventValue::String(text) => EventValue::String(pattern::apply(patterns, text)),
        EventValue::Object(mapping) => walk_object(mapping, patterns, path),
        EventValue::Array(items) => walk_array(items, patterns, path),
        EventValue::Null => EventValue::Null,
        EventValue::Bool(flag) => EventValue::Bool(*flag),
        EventValue::Number(number) => EventValue::Number(number.clone()),
    }
}

fn walk_object(
    mapping: &BTreeMap<String, EventValue>,
    patterns: &Patterns,
    path: &[String],
) -> EventValue {
    let unscrubbed = is_stack_frame_path(path);
    let mut output = BTreeMap::new();
    for (key, item) in mapping {
        let scrubbed = if unscrubbed && SOURCE_CONTEXT_KEYS.contains(&key.as_str()) {
            item.clone()
        } else {
            let mut child_path = path.to_vec();
            child_path.push(key.clone());
            walk(item, patterns, &child_path)
        };
        output.insert(key.clone(), scrubbed);
    }
    EventValue::Object(output)
}

fn walk_array(items: &[EventValue], patterns: &Patterns, path: &[String]) -> EventValue {
    let mut output = Vec::with_capacity(items.len());
    for item in items {
        let mut child_path = path.to_vec();
        child_path.push("*".to_owned());
        output.push(walk(item, patterns, &child_path));
    }
    EventValue::Array(output)
}

mod tests;
