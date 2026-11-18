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

fn json_map(value: Value) -> Option<Map<String, Value>> {
    value.as_object().cloned()
}

pub fn parse_sse_data_line(raw_line: &str) -> Option<Map<String, Value>> {
    let line = raw_line.trim();
    let data = line.strip_prefix("data:")?.trim();
    json_map(serde_json::from_str(data).ok()?)
}

pub fn iter_sse_events(item: &SseChunk) -> Vec<Map<String, Value>> {
    match item {
        SseChunk::Event(event) => vec![event.clone()],
        SseChunk::Bytes(bytes) => String::from_utf8_lossy(bytes)
            .split("\n\n")
            .flat_map(|block| block.lines().filter_map(parse_sse_data_line))
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

fn event_kind(block: &str) -> Option<&str> {
    block
        .lines()
        .filter_map(|line| line.trim().strip_prefix("event:").map(str::trim))
        .last()
}

fn data_line(block: &str) -> Option<&str> {
    block
        .lines()
        .filter_map(|line| line.trim().strip_prefix("data:").map(str::trim))
        .last()
}

pub fn extract_text_from_sse(bytes: &[u8]) -> String {
    let Ok(source) = std::str::from_utf8(bytes) else {
        return String::new();
    };
    let mut text = String::new();
    for block in source
        .split("\n\n")
        .filter(|block| !block.trim().is_empty())
    {
        if event_kind(block) != Some("content_block_delta") {
            continue;
        }
        let Some(data) = data_line(block) else {
            continue;
        };
        if let Ok(value) = serde_json::from_str::<Value>(data)
            && value.pointer("/delta/type").and_then(Value::as_str) == Some("text_delta")
        {
            text.push_str(
                value
                    .pointer("/delta/text")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            );
        }
    }
    text
}

pub fn streaming_string_so_far(items: &[SseChunk]) -> String {
    items
        .iter()
        .map(|item| match item {
            SseChunk::Bytes(bytes) => extract_text_from_sse(bytes),
            SseChunk::Event(event)
                if event.pointer("/delta/type").and_then(Value::as_str) == Some("text_delta") =>
            {
                event
                    .pointer("/delta/text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned()
            }
            SseChunk::Event(_) | SseChunk::Other => String::new(),
        })
        .collect()
}

pub fn streaming_has_ended(items: &[SseChunk]) -> bool {
    items.iter().any(|item| match item {
        SseChunk::Bytes(bytes) => String::from_utf8_lossy(bytes).split("\n\n").any(|block| {
            event_kind(block) == Some("message_delta")
                && data_line(block)
                    .and_then(|data| serde_json::from_str::<Value>(data).ok())
                    .and_then(|value| value.pointer("/delta/stop_reason").cloned())
                    .is_some_and(|value| !value.is_null())
        }),
        SseChunk::Event(event) => {
            event.get("type").and_then(Value::as_str) == Some("message_delta")
                && event
                    .pointer("/delta/stop_reason")
                    .is_some_and(|value| !value.is_null())
        }
        SseChunk::Other => false,
    })
}

pub fn streamed_tool_use_fingerprints(items: &[SseChunk]) -> Vec<String> {
    items
        .iter()
        .flat_map(iter_sse_events)
        .filter(|event| {
            event.get("type").and_then(Value::as_str) == Some("content_block_start")
                && event.pointer("/content_block/type").and_then(Value::as_str) == Some("tool_use")
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
