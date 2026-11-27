use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::{Object, StreamItem};

const DATA_PREFIX: &str = "data:";

#[must_use]
pub fn anthropic_stream_chunk_events(item: &StreamItem) -> Vec<Object> {
    match item {
        StreamItem::Json(GuardrailValue::Object(event)) => vec![event.clone()],
        StreamItem::Json(GuardrailValue::Text(chunk)) => events_from_text(chunk),
        StreamItem::Bytes(bytes) => events_from_text(&decode_replacing_invalid(bytes)),
        StreamItem::Json(_) => Vec::new(),
    }
}

#[must_use]
pub fn stream_item_value(item: &StreamItem) -> GuardrailValue {
    match item {
        StreamItem::Json(value) => value.clone(),
        StreamItem::Bytes(bytes) => GuardrailValue::Text(decode_replacing_invalid(bytes)),
    }
}

// Python's errors="replace": one U+FFFD per invalid sequence. String::from_utf8_lossy does the same but clippy.toml bans it.
pub fn decode_replacing_invalid(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        text.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            text.push('\u{FFFD}');
        }
    }
    text
}

// SSE lines end only at CR and LF, so U+001C to U+001F reach the trim and Python's strip removes them; a payload that is no JSON object parses to nothing.
fn events_from_text(chunk: &str) -> Vec<Object> {
    let is_blank =
        |character: char| character.is_whitespace() || matches!(character, '\u{1c}'..='\u{1f}');
    let mut events = Vec::new();
    for line in chunk.split(['\r', '\n']) {
        let Some(raw_payload) = line.trim_matches(is_blank).strip_prefix(DATA_PREFIX) else {
            continue;
        };
        let payload = raw_payload.trim_matches(is_blank);
        if let Ok(event) = serde_json::from_str::<Object>(payload) {
            events.push(event);
        }
    }
    events
}

mod tests;
