use super::sse_stream::{event_type, joined_sse_stream, parsed_sse_events};
use crate::guardrail_message_scope::StreamItem;

mod tests;

const ANTHROPIC_EVENT_TYPES: [&str; 8] = [
    "message_start",
    "message_delta",
    "message_stop",
    "content_block_start",
    "content_block_delta",
    "content_block_stop",
    "ping",
    "error",
];

// Google's streamGenerateContent is raw too, so the surface is decided by the event types present.
#[must_use]
pub fn is_anthropic_sse_stream(all_chunks: &[StreamItem]) -> bool {
    let Some(sse_stream) = joined_sse_stream(all_chunks) else {
        return false;
    };
    parsed_sse_events(&sse_stream)
        .iter()
        .filter_map(event_type)
        .any(|kind| ANTHROPIC_EVENT_TYPES.contains(&kind))
}
