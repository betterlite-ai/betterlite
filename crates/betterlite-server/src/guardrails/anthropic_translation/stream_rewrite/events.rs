use serde_json::{Map, Value};

use super::line::rewrite_sse_events;
use super::rewriter::EventRewriter;
use crate::guardrails::anthropic_translation::sse_events::SseChunk;

// LiteLLM's _rewritten_event: the event with one nested section's field replaced, or None when nothing was rewritten.
#[must_use]
pub fn rewritten_event(
    event: &Map<String, Value>,
    rewriter: &mut EventRewriter,
) -> Option<Map<String, Value>> {
    // the rewriter advances its own state even when the section turns out not to be an object
    let rewrite = rewriter.rewrite(event)?;
    let section = event.get(rewrite.section)?.as_object()?;
    let mut updated = section.clone();
    updated.insert(rewrite.field.to_owned(), Value::String(rewrite.value));
    let mut rewritten = event.clone();
    rewritten.insert(rewrite.section.to_owned(), Value::Object(updated));
    Some(rewritten)
}

// LiteLLM's _rewrite_ended_stream_events: every buffered chunk is rewritten in place, in stream order.
pub fn rewrite_ended_stream_events(
    responses_so_far: &mut [SseChunk],
    rewriter: &mut EventRewriter,
) {
    for chunk in responses_so_far.iter_mut() {
        match chunk {
            SseChunk::Bytes(bytes) => {
                let rewritten = rewrite_sse_events(bytes, rewriter);
                *bytes = rewritten;
            }
            SseChunk::Event(event) => {
                if let Some(rewritten) = rewritten_event(event, rewriter) {
                    *event = rewritten;
                }
            }
            SseChunk::Other => {}
        }
    }
}
