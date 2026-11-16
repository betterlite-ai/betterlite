use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::api::responses_tool_namespaces::values::Fields;
use betterlite_models::decisions::Json;
use betterlite_streaming::passthrough_reconstruction::events::extract_sse_data;

use crate::guardrail_message_scope::StreamItem;

// A relayed chunk is raw when it is the text or the bytes of a frame, and parsed when it is an object.
#[must_use]
pub fn raw_chunk_bytes(chunk: &StreamItem) -> Option<&[u8]> {
    match chunk {
        StreamItem::Bytes(bytes) => Some(bytes),
        StreamItem::Json(GuardrailValue::Text(text)) => Some(text.as_bytes()),
        StreamItem::Json(_) => None,
    }
}

// The decoder is incremental: a multi-byte character cut off at the end waits for the next read, and any other bad byte fails the join.
#[must_use]
pub fn joined_sse_stream(chunks: &[StreamItem]) -> Option<String> {
    let mut raw: Vec<u8> = chunks
        .iter()
        .filter_map(raw_chunk_bytes)
        .flatten()
        .copied()
        .collect();
    if let Err(error) = str::from_utf8(&raw) {
        if error.error_len().is_some() {
            return None;
        }
        raw.truncate(error.valid_up_to());
    }
    String::from_utf8(raw).ok()
}

// Only the first data line of each event is read, and one that is not JSON is skipped.
#[must_use]
pub fn parsed_sse_events(sse_stream: &str) -> Vec<Json> {
    sse_stream
        .split("\n\n")
        .map(str::trim)
        .filter_map(extract_sse_data)
        .collect()
}

#[must_use]
pub fn event_type(event: &Json) -> Option<&str> {
    let Json::Object(fields) = event else {
        return None;
    };
    match fields.get("type") {
        Some(Json::Text(kind)) => Some(kind),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

// The first message_start event whose message is an object.
#[must_use]
pub fn anthropic_message_start(events: &[Json]) -> Option<&Fields> {
    events.iter().find_map(|event| {
        let Json::Object(fields) = event else {
            return None;
        };
        match (event_type(event), fields.get("message")) {
            (Some("message_start"), Some(Json::Object(message))) => Some(message),
            _ => None,
        }
    })
}
