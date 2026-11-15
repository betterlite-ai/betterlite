#![expect(
    clippy::disallowed_types,
    reason = "Responses stream events are heterogeneous provider JSON with no closed schema"
)]

pub mod error;
pub mod records;
pub mod stream;
pub mod translate;

mod fields;
mod passthrough;
mod terminal;
mod tests;
mod text;
mod tool_call;
mod usage;

use std::str::from_utf8;

use serde_json::Value;

use crate::api::api_boundaries::{ApiBoundariesInput, ApiLimits, EventBoundary};
use error::ResponsesChatStreamError;
use records::PaddingChunk;

const DATA_PREFIX: &str = "data:";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodedEventLine {
    Padding(PaddingChunk),
    Event(Value),
}

// _handle_string_chunk before chunk_parser: an empty line and an `event:` line are no events at all
pub fn decode_event_line(
    bytes: &[u8],
    limits: &ApiLimits,
) -> Result<DecodedEventLine, ResponsesChatStreamError> {
    limits.check(ApiBoundariesInput::Event(EventBoundary {
        known_bytes: bytes.len(),
    }))?;
    let text = from_utf8(bytes)?;
    if text.is_empty() || text.starts_with("event:") {
        return Ok(DecodedEventLine::Padding(PaddingChunk::default()));
    }
    let payload = text.find(DATA_PREFIX).map_or(text, |index| {
        text.get(index.saturating_add(DATA_PREFIX.len())..)
            .unwrap_or_default()
    });
    let event = serde_json::from_str::<Value>(payload).map_err(|source| {
        ResponsesChatStreamError::Json {
            message: text::scanned_error(payload).unwrap_or_else(|| source.to_string()),
            source,
        }
    })?;
    Ok(DecodedEventLine::Event(event))
}
