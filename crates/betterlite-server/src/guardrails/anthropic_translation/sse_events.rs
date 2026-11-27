#![expect(
    clippy::disallowed_types,
    reason = "provider SSE JSON is an open wire boundary"
)]
#![cfg_attr(
    test,
    expect(
        clippy::default_numeric_fallback,
        clippy::disallowed_methods,
        reason = "serde_json fixtures compare JSON numbers and construct provider objects"
    )
)]

use crate::guardrails::responses_translation::block_events::guardrail_json::to_guardrail;
use betterlite_guardrails::guardrail_message_scope::StreamItem;
use betterlite_guardrails::guardrail_message_scope::fingerprint::stream_item_fingerprint;
use betterlite_guardrails::guardrail_message_scope::stream_chunk_events::decode_replacing_invalid;
use betterlite_guardrails::guardrail_translation::StreamingScanKey;
use serde_json::{Map, Value};
use std::borrow::Cow;
use std::str;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SseChunk {
    Bytes(Vec<u8>),
    Event(Map<String, Value>),
    Other,
}

// Python's str.strip also drops U+001C to U+001F, which Rust's trim leaves in place.
fn trim_sse(text: &str) -> &str {
    text.trim_matches(|character: char| {
        character.is_whitespace() || matches!(character, '\u{1c}'..='\u{1f}')
    })
}

// One stream item as the Anthropic handler reads it: a parsed event, a raw frame's bytes, or a value it leaves alone
#[must_use]
pub fn stream_item_chunk(item: &StreamItem) -> Result<SseChunk, serde_json::Error> {
    match item {
        StreamItem::Json(value) => match serde_json::to_value(value)? {
            Value::Object(fields) => Ok(SseChunk::Event(fields)),
            Value::Null
            | Value::Bool(_)
            | Value::Number(_)
            | Value::String(_)
            | Value::Array(_) => Ok(SseChunk::Other),
        },
        StreamItem::Bytes(bytes) => Ok(SseChunk::Bytes(bytes.clone())),
    }
}

#[must_use]
pub fn parse_sse_data_line(raw_line: &str) -> Option<Map<String, Value>> {
    let line = trim_sse(raw_line);
    let data = trim_sse(line.strip_prefix("data:")?);
    match serde_json::from_str::<Value>(data) {
        Ok(Value::Object(fields)) => Some(fields),
        Ok(_) | Err(_) => None,
    }
}

#[must_use]
pub fn iter_sse_events(item: &SseChunk) -> Vec<Map<String, Value>> {
    match item {
        SseChunk::Event(event) => vec![event.clone()],
        SseChunk::Bytes(bytes) => decode_replacing_invalid(bytes)
            .split("\n\n")
            .flat_map(|block| block.split('\n').filter_map(parse_sse_data_line))
            .collect(),
        SseChunk::Other => Vec::new(),
    }
}

#[must_use]
pub fn content_block_state(items: &[SseChunk]) -> (Option<i64>, Option<i64>) {
    let mut open = Vec::<i64>::new();
    let mut highest = None;
    for event in items.iter().flat_map(iter_sse_events) {
        let Some(index) = event.get("index").and_then(Value::as_i64) else {
            continue;
        };
        match event.get("type").and_then(Value::as_str) {
            Some("content_block_start") => {
                highest = Some(highest.map_or(index, |previous: i64| previous.max(index)));
                open.push(index);
            }
            Some("content_block_stop") => {
                open.retain(|open_index| *open_index != index);
            }
            _ => {}
        }
    }
    (open.into_iter().max(), highest)
}

fn last_prefixed_line<'block>(block: &'block str, prefix: &str) -> Option<&'block str> {
    trim_sse(block)
        .split('\n')
        .filter_map(|line| line.strip_prefix(prefix).map(trim_sse))
        .next_back()
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

#[must_use]
pub fn extract_text_from_sse(bytes: &[u8]) -> String {
    let Ok(source) = str::from_utf8(bytes) else {
        tracing::error!("Error extracting text from SSE");
        return String::new();
    };
    let mut text = String::new();
    for block in source.split("\n\n") {
        if trim_sse(block).is_empty() {
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

#[must_use]
pub fn streaming_string_so_far(items: &[SseChunk]) -> String {
    let mut text = String::new();
    for item in items {
        let delta = match item {
            SseChunk::Bytes(bytes) => Cow::Owned(extract_text_from_sse(bytes)),
            SseChunk::Event(event) => Cow::Borrowed(text_delta(event).unwrap_or("")),
            SseChunk::Other => continue,
        };
        text.push_str(&delta);
    }
    text
}

#[must_use]
pub fn streaming_has_ended(items: &[SseChunk]) -> bool {
    items.iter().any(|item| match item {
        SseChunk::Bytes(bytes) => has_ended_bytes(bytes),
        SseChunk::Event(event) => {
            event.get("type").and_then(Value::as_str) == Some("message_delta")
                && stop_reason_is_set(event)
        }
        SseChunk::Other => false,
    })
}

fn has_ended_bytes(bytes: &[u8]) -> bool {
    let Ok(source) = str::from_utf8(bytes) else {
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

#[must_use]
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
            stream_item_fingerprint(&StreamItem::Json(to_guardrail(block))).ok()
        })
        .collect()
}

#[must_use]
pub fn streaming_scan_key(items: &[SseChunk]) -> StreamingScanKey {
    let stream_ended = streaming_has_ended(items);
    let fingerprints = streamed_tool_use_fingerprints(items);
    let tool_calls_in_flight = !fingerprints.is_empty() && !stream_ended;
    let tool_calls = if stream_ended {
        fingerprints
    } else {
        Vec::new()
    };
    StreamingScanKey {
        texts: vec![streaming_string_so_far(items)],
        tool_calls,
        stream_ended,
        tool_calls_in_flight,
    }
}

mod tests;
