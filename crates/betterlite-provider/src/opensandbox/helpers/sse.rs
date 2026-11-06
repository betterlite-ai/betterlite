use std::collections::BTreeMap;

use betterlite_models::sandbox::{CodeExecutionResult, ExecutionError};
use serde_json::value::{RawValue, to_raw_value};

pub type Message = BTreeMap<String, Box<RawValue>>;

const IGNORED_PREFIXES: [&str; 4] = [":", "event:", "id:", "retry:"];

pub fn parse_sse_line(line: &str) -> Option<Message> {
    let stripped = line.trim();
    if stripped.is_empty() {
        return None;
    }
    let ignored = IGNORED_PREFIXES
        .iter()
        .any(|prefix| stripped.starts_with(prefix));
    if ignored {
        return None;
    }
    let data = stripped.strip_prefix("data:").map_or(stripped, str::trim);
    if data.is_empty() {
        return None;
    }
    let parsed: Message = serde_json::from_str(data).ok()?;
    if !parsed.contains_key("type") && parsed.contains_key("code") && parsed.contains_key("message")
    {
        return code_and_message_error(&parsed);
    }
    Some(parsed)
}

#[must_use]
pub fn parse_lines(lines: &[String]) -> CodeExecutionResult {
    let messages: Vec<Message> = lines
        .iter()
        .filter_map(|line| parse_sse_line(line))
        .collect();
    let mut result = CodeExecutionResult::default();
    for message in &messages {
        let kind = message
            .get("type")
            .and_then(|raw| serde_json::from_str::<String>(raw.get()).ok());
        match kind.as_deref() {
            Some("stdout") => result.stdout.push_str(&text_of(message)),
            Some("stderr") => result.stderr.push_str(&text_of(message)),
            Some("result") => result.results.push(normalize_result(message)),
            Some("error") if result.error.is_none() => result.error = normalize_error(message),
            Some("execution_count") if result.execution_count.is_none() => {
                result.execution_count = message.get("execution_count").and_then(|raw| as_int(raw));
            }
            _ => {}
        }
    }
    result
}

#[must_use]
pub fn normalize_result(message: &Message) -> Message {
    // LiteLLM prefers a nested results object and otherwise copies every field but the envelope
    message
        .get("results")
        .and_then(|raw| serde_json::from_str::<Message>(raw.get()).ok())
        .unwrap_or_else(|| {
            message
                .iter()
                .filter(|(key, _)| {
                    !matches!(key.as_str(), "type" | "timestamp" | "execution_count")
                })
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        })
}

#[must_use]
pub fn normalize_error(message: &Message) -> Option<ExecutionError> {
    let nested = message
        .get("error")
        .and_then(|raw| serde_json::from_str::<Message>(raw.get()).ok());
    let (source, name_keys, value_keys): (&Message, &[&str], &[&str]) = nested
        .as_ref()
        .map_or((message, &["name"], &["value", "text"]), |error| {
            (error, &["ename", "name"], &["evalue", "value"])
        });
    Some(ExecutionError {
        name: Some(first_raw(source, name_keys).or_else(|| to_raw_value("").ok())?),
        value: Some(first_raw(source, value_keys).or_else(|| to_raw_value("").ok())?),
        traceback: Some(
            first_raw(source, &["traceback"])
                .or_else(|| to_raw_value(&Vec::<String>::new()).ok())?,
        ),
    })
}

#[must_use]
pub fn as_int(raw: &RawValue) -> Option<i64> {
    if let Ok(value) = serde_json::from_str::<i64>(raw.get()) {
        return Some(value);
    }
    let text = serde_json::from_str::<String>(raw.get()).ok()?;
    text.parse::<i64>().ok()
}

pub(super) fn raw_text(raw: &RawValue) -> String {
    let Ok(text) = serde_json::from_str::<String>(raw.get()) else {
        return raw.get().to_owned();
    };
    text
}

fn first_raw(source: &Message, keys: &[&str]) -> Option<Box<RawValue>> {
    keys.iter().find_map(|key| {
        let value = source.get(*key)?;
        (value.get() != "null").then_some(Box::clone(value))
    })
}

fn text_of(message: &Message) -> String {
    message
        .get("text")
        .map_or_else(String::new, |raw| raw_text(raw))
}

fn code_and_message_error(parsed: &Message) -> Option<Message> {
    let code = raw_text(parsed.get("code")?);
    let message = raw_text(parsed.get("message")?);
    let document = format!(
        r#"{{"type":"error","error":{{"ename":{},"evalue":{},"traceback":[]}}}}"#,
        serde_json::to_string(&code).ok()?,
        serde_json::to_string(&message).ok()?
    );
    serde_json::from_str(&document).ok()
}
