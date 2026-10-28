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

pub fn scrub_event(event: &EventValue, send_default_pii: bool) -> Result<EventValue, PatternError> {
    let patterns = pattern::build(send_default_pii)?;
    Ok(walk(event, &patterns, &[]))
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
    let unscrubbed = STACK_FRAME_PATHS.iter().any(|candidate| {
        candidate.len() == path.len()
            && candidate
                .iter()
                .zip(path)
                .all(|(expected, actual)| *expected == actual.as_str())
    });
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
    let mut child_path = path.to_vec();
    child_path.push("*".to_owned());
    let scrubbed = items.iter().map(|item| walk(item, patterns, &child_path));
    EventValue::Array(scrubbed.collect())
}

mod tests;
