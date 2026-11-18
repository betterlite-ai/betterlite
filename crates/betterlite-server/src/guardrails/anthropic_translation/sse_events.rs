#![expect(
    clippy::disallowed_types,
    reason = "provider SSE JSON is an open wire boundary"
)]

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_message_scope::StreamItem;
use betterlite_guardrails::guardrail_message_scope::fingerprint::stream_item_fingerprint;
use betterlite_guardrails::guardrail_translation::StreamingScanKey;
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq)]
pub enum SseChunk {
    Bytes(Vec<u8>),
    Event(Map<String, Value>),
    Other,
}

fn guardrail_value(value: &Value) -> GuardrailValue {
    match value {
        Value::Null => GuardrailValue::Null,
        Value::Bool(value) => GuardrailValue::Boolean(*value),
        Value::Number(value) => value.as_i64().map_or_else(
            || {
                value.as_u64().map_or_else(
                    || GuardrailValue::Decimal(value.as_f64().unwrap_or_default()),
                    GuardrailValue::UnsignedInteger,
                )
            },
            GuardrailValue::SignedInteger,
        ),
        Value::String(value) => GuardrailValue::Text(value.clone()),
        Value::Array(values) => GuardrailValue::Array(values.iter().map(guardrail_value).collect()),
        Value::Object(values) => GuardrailValue::Object(
            values
                .iter()
                .map(|(key, value)| (key.clone(), guardrail_value(value)))
                .collect(),
        ),
    }
}

// Python's str.strip also drops U+001C to U+001F, which Rust's trim leaves in place.
fn python_trim(text: &str) -> &str {
    text.trim_matches(|character: char| {
        character.is_whitespace() || matches!(character, '\u{1c}'..='\u{1f}')
    })
}

// Python's errors="replace": one U+FFFD per invalid sequence. String::from_utf8_lossy does the same but clippy.toml bans it.
fn utf8_replacing_invalid(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        text.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            text.push('\u{FFFD}');
        }
    }
    text
}

pub fn parse_sse_data_line(raw_line: &str) -> Option<Map<String, Value>> {
    let line = python_trim(raw_line);
    let data = python_trim(line.strip_prefix("data:")?);
    match serde_json::from_str::<Value>(data) {
        Ok(Value::Object(fields)) => Some(fields),
        Ok(_) | Err(_) => None,
    }
}

pub fn iter_sse_events(item: &SseChunk) -> Vec<Map<String, Value>> {
    match item {
        SseChunk::Event(event) => vec![event.clone()],
        SseChunk::Bytes(bytes) => utf8_replacing_invalid(bytes)
            .split("\n\n")
            .flat_map(|block| block.split('\n').filter_map(parse_sse_data_line))
            .collect(),
        SseChunk::Other => Vec::new(),
    }
}

pub fn content_block_state(items: &[SseChunk]) -> (Option<i64>, Option<i64>) {
    let mut open = Vec::<i64>::new();
    let mut highest = None;
    for event in items.iter().flat_map(iter_sse_events) {
        let index = event.get("index").and_then(Value::as_i64);
        match event.get("type").and_then(Value::as_str) {
            Some("content_block_start") => {
                if let Some(index) = index {
                    highest = Some(highest.map_or(index, |previous: i64| previous.max(index)));
                    open.push(index);
                }
            }
            Some("content_block_stop") => {
                if let Some(index) = index {
                    open.retain(|open_index| *open_index != index);
                }
            }
            _ => {}
        }
    }
    (open.into_iter().max(), highest)
}

fn last_prefixed_line<'block>(block: &'block str, prefix: &str) -> Option<&'block str> {
    python_trim(block)
        .split('\n')
        .filter_map(|line| line.strip_prefix(prefix).map(python_trim))
        .last()
}

fn text_delta(event: &Map<String, Value>) -> Option<&str> {
    let delta = event.get("delta").and_then(Value::as_object)?;
    if delta.get("type").and_then(Value::as_str) != Some("text_delta") {
        return None;
    }
    delta.get("text").and_then(Value::as_str)
}

fn stop_reason_is_set(event: &Map<String, Value>) -> bool {
    event
        .get("delta")
        .and_then(Value::as_object)
        .and_then(|delta| delta.get("stop_reason"))
        .is_some_and(|reason| !reason.is_null())
}

fn event_has_ended(event: &Map<String, Value>) -> bool {
    event.get("type").and_then(Value::as_str) == Some("message_delta") && stop_reason_is_set(event)
}

pub fn extract_text_from_sse(bytes: &[u8]) -> String {
    let Ok(source) = std::str::from_utf8(bytes) else {
        tracing::error!("Error extracting text from SSE");
        return String::new();
    };
    let mut text = String::new();
    for block in source.split("\n\n") {
        if python_trim(block).is_empty() {
            continue;
        }
        if last_prefixed_line(block, "event:") != Some("content_block_delta") {
            continue;
        }
        let Some(data) = last_prefixed_line(block, "data:") else {
            continue;
        };
        if data.is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(data) {
            Ok(Value::Object(event)) => {
                if let Some(delta) = text_delta(&event) {
                    text.push_str(delta);
                }
            }
            Ok(_) => {}
            Err(_) => tracing::warn!(data = data, "Failed to parse JSON from SSE data"),
        }
    }
    text
}

pub fn streaming_string_so_far(items: &[SseChunk]) -> String {
    items
        .iter()
        .map(|item| match item {
            SseChunk::Bytes(bytes) => extract_text_from_sse(bytes),
            SseChunk::Event(event) => text_delta(event).unwrap_or_default().to_owned(),
            SseChunk::Other => String::new(),
        })
        .collect()
}

pub fn streaming_has_ended(items: &[SseChunk]) -> bool {
    items.iter().any(|item| match item {
        SseChunk::Bytes(bytes) => has_ended_bytes(bytes),
        SseChunk::Event(event) => event_has_ended(event),
        SseChunk::Other => false,
    })
}

fn has_ended_bytes(bytes: &[u8]) -> bool {
    let Ok(source) = std::str::from_utf8(bytes) else {
        tracing::error!("Error checking streaming end in SSE");
        return false;
    };
    for block in source.split("\n\n") {
        if last_prefixed_line(block, "event:") != Some("message_delta") {
            continue;
        }
        let Some(data) = last_prefixed_line(block, "data:") else {
            continue;
        };
        if data.is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(data) {
            Ok(Value::Object(event)) if stop_reason_is_set(&event) => return true,
            Ok(_) => {}
            Err(_) => tracing::warn!(data = data, "Failed to parse JSON from SSE data"),
        }
    }
    false
}

pub fn streamed_tool_use_fingerprints(items: &[SseChunk]) -> Vec<String> {
    items
        .iter()
        .flat_map(iter_sse_events)
        .filter(|event| {
            event.get("type").and_then(Value::as_str) == Some("content_block_start")
                && event
                    .get("content_block")
                    .and_then(|block| block.get("type"))
                    .and_then(Value::as_str)
                    == Some("tool_use")
        })
        .filter_map(|event| {
            let block = event.get("content_block")?;
            stream_item_fingerprint(&StreamItem::Json(guardrail_value(block))).ok()
        })
        .collect()
}

pub fn streaming_scan_key(items: &[SseChunk]) -> StreamingScanKey {
    let stream_ended = streaming_has_ended(items);
    let fingerprints = streamed_tool_use_fingerprints(items);
    let tool_calls = if stream_ended {
        fingerprints.clone()
    } else {
        Vec::new()
    };
    StreamingScanKey {
        texts: vec![streaming_string_so_far(items)],
        tool_calls,
        stream_ended,
        tool_calls_in_flight: !fingerprints.is_empty() && !stream_ended,
    }
}

#[cfg(test)]
mod tests;
