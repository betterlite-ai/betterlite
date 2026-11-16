use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::guardrail_message_scope::{Object, StreamItem};

// LiteLLM's is_raw_sse_stream: a str or bytes chunk is unparsed wire data, never a chunk object
#[must_use]
pub(crate) fn is_raw_item(item: &StreamItem) -> bool {
    matches!(
        item,
        StreamItem::Bytes(_) | StreamItem::Json(GuardrailValue::Text(_))
    )
}

// LiteLLM's _joined_sse_stream: every raw chunk joined in order, decoded the way an incremental UTF-8 decoder does
#[must_use]
pub(crate) fn join_raw_items(items: &[StreamItem]) -> Option<String> {
    let mut bytes = Vec::new();
    for item in items {
        match item {
            StreamItem::Bytes(chunk) => bytes.extend_from_slice(chunk),
            StreamItem::Json(GuardrailValue::Text(chunk)) => {
                bytes.extend_from_slice(chunk.as_bytes());
            }
            StreamItem::Json(_) => {}
        }
    }
    decode_incremental(&bytes)
}

// An incomplete multibyte sequence at the very end is still pending in Python's decoder, so it does not fail the join.
fn decode_incremental(bytes: &[u8]) -> Option<String> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Some(text.to_owned()),
        Err(error) => {
            if error.error_len().is_some() {
                return None;
            }
            let valid = bytes.get(..error.valid_up_to())?;
            let text = std::str::from_utf8(valid).ok()?;
            Some(text.to_owned())
        }
    }
}

// LiteLLM's _split_sse_chunk_into_events with _extract_sse_data: each event's first data line, parsed as a JSON object
#[must_use]
pub(crate) fn sse_events(text: &str) -> Vec<Object> {
    let mut events = Vec::new();
    for frame in text.split("\n\n") {
        let frame = frame.trim();
        if frame.is_empty() {
            continue;
        }
        if let Some(event) = first_data_line(frame) {
            events.push(event);
        }
    }
    events
}

fn first_data_line(frame: &str) -> Option<Object> {
    for line in frame.lines() {
        let line = line.trim();
        let Some(payload) = line.strip_prefix("data:") else {
            continue;
        };
        let payload = payload.trim();
        if payload.is_empty() || payload == "[DONE]" {
            return None;
        }
        return serde_json::from_str::<Object>(payload).ok();
    }
    None
}
