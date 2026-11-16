use betterlite_guardrails::guardrail_message_scope::StreamItem;
use betterlite_models::decisions::Json;

use super::sse_stream::{event_type, joined_sse_stream, parsed_sse_events, raw_chunk_bytes};

mod tests;

fn is_error_event(event: &Json) -> bool {
    let Json::Object(fields) = event else {
        return false;
    };
    event_type(event) == Some("error") || matches!(fields.get("error"), Some(Json::Object(_)))
}

// A stream that also holds typed chunks has content to scan, so only a stream of nothing but error frames counts.
#[must_use]
pub fn is_sse_error_stream(all_chunks: &[StreamItem]) -> bool {
    if all_chunks
        .iter()
        .any(|chunk| raw_chunk_bytes(chunk).is_none())
    {
        return false;
    }
    let Some(sse_stream) = joined_sse_stream(all_chunks) else {
        return false;
    };
    let events = parsed_sse_events(&sse_stream);
    !events.is_empty() && events.iter().all(is_error_event)
}
