use super::sse_stream::raw_chunk_bytes;
use crate::guardrail_message_scope::StreamItem;

mod tests;

// Raw means some chunk is unparsed text or bytes rather than an assembled stream chunk.
#[must_use]
pub fn is_raw_sse_stream(all_chunks: &[StreamItem]) -> bool {
    all_chunks
        .iter()
        .any(|chunk| raw_chunk_bytes(chunk).is_some())
}
