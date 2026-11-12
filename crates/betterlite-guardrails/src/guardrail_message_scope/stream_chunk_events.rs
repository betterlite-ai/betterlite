use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::{Object, StreamItem};

const DATA_PREFIX: &str = "data:";
const DONE_MARKER: &str = "[DONE]";

#[must_use]
pub fn anthropic_stream_chunk_events(item: &StreamItem) -> Vec<Object> {
    match item {
        StreamItem::Json(GuardrailValue::Object(event)) => vec![event.clone()],
        StreamItem::Json(GuardrailValue::Text(chunk)) => events_from_text(chunk),
        StreamItem::Bytes(bytes) => events_from_text(&decode_replacing_invalid(bytes)),
        StreamItem::Json(_) => Vec::new(),
    }
}

fn decode_replacing_invalid(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        text.push_str(chunk.valid());
        if !chunk.invalid().is_empty() {
            text.push('\u{FFFD}');
        }
    }
    text
}

// The same line breaks and blanks Python's splitlines and strip use.
fn events_from_text(chunk: &str) -> Vec<Object> {
    let is_blank = |character: char| character.is_whitespace() || character == '\u{1f}';
    let is_line_break = |character: char| {
        matches!(
            character,
            '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{1c}'
                ..='\u{1e}' | '\u{85}' | '\u{2028}' | '\u{2029}'
        )
    };
    let mut events = Vec::new();
    for line in chunk.split(is_line_break) {
        let Some(raw_payload) = line.trim_matches(is_blank).strip_prefix(DATA_PREFIX) else {
            continue;
        };
        let payload = raw_payload.trim_matches(is_blank);
        if payload.is_empty() || payload == DONE_MARKER {
            continue;
        }
        if let Ok(event) = serde_json::from_str::<Object>(payload) {
            events.push(event);
        }
    }
    events
}

mod tests;
