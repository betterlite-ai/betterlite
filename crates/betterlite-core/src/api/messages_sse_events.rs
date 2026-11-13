use std::collections::BTreeSet;

use betterlite_models::decisions::Json;

use crate::api::api_boundaries::{ApiBoundariesInput, ApiLimitError, ApiLimits, EventBoundary};

use chunk::{ChunkFlag, DecodedMessagesChunk};
use raw_bytes::{data_line_payloads, has_event_line, is_ping_bytes};

pub mod chunk;
mod raw_bytes;
pub mod render;
pub mod stream;
mod tests;

pub fn decode_messages_sse_chunk(
    bytes: &[u8],
    limits: &ApiLimits,
) -> Result<DecodedMessagesChunk, ApiLimitError> {
    limits.check(ApiBoundariesInput::Event(EventBoundary {
        known_bytes: bytes.len(),
    }))?;
    let mut flags = BTreeSet::new();
    if has_event_line(bytes, b"event: message_stop") {
        flags.insert(ChunkFlag::MessageStop);
    }
    if is_ping_bytes(bytes) {
        flags.insert(ChunkFlag::Ping);
    }
    if has_event_line(bytes, b"event: content_block_delta") {
        flags.insert(ChunkFlag::ContentDelta);
    }
    Ok(DecodedMessagesChunk::new(flags, data_line_payloads(bytes)))
}

#[must_use]
pub fn decode_messages_sse_value(value: &Json) -> DecodedMessagesChunk {
    let mut flags = BTreeSet::new();
    if let Json::Object(fields) = value {
        match chunk::field_text(fields, "type") {
            Some("message_stop") => {
                flags.insert(ChunkFlag::MessageStop);
            }
            Some("ping") => {
                flags.insert(ChunkFlag::Ping);
            }
            Some("content_block_delta") => {
                flags.insert(ChunkFlag::ContentDelta);
            }
            Some(_) | None => {}
        }
    }
    DecodedMessagesChunk::new(flags, vec![value.clone()])
}
