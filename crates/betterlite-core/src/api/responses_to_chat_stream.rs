#![expect(
    clippy::disallowed_types,
    reason = "Responses stream events are heterogeneous provider JSON with no closed schema"
)]

pub mod error;
pub mod records;
pub mod stream;
pub mod translate;

mod passthrough;
mod terminal;
mod tests;
mod text;
mod tool_call;
mod usage;

use serde_json::Value;

use crate::api::api_boundaries::{ApiBoundariesInput, ApiLimits, EventBoundary};
use error::ResponsesChatStreamError;
use records::PaddingChunk;

const DATA_PREFIX: &str = "data:";

#[derive(Debug, Clone, PartialEq)]
pub enum DecodedEventLine {
    Padding(PaddingChunk),
    Event(Value),
}

// _handle_string_chunk before chunk_parser: an empty line and an `event:` line are no events at all
pub fn decode_responses_chat_event_line(
    bytes: &[u8],
    limits: &ApiLimits,
) -> Result<DecodedEventLine, ResponsesChatStreamError> {
    limits
        .check(ApiBoundariesInput::Event(EventBoundary {
            known_bytes: bytes.len(),
        }))
        .map_err(ResponsesChatStreamError::from)?;
    let text =
        std::str::from_utf8(bytes).map_err(|source| ResponsesChatStreamError::Utf8 { source })?;
    if text.is_empty() || text.starts_with("event:") {
        return Ok(DecodedEventLine::Padding(PaddingChunk {
            text: String::new(),
            tool_use: None,
            is_finished: false,
            finish_reason: String::new(),
            usage: None,
        }));
    }
    let payload = match text.find(DATA_PREFIX) {
        Some(index) => text
            .get(index.saturating_add(DATA_PREFIX.len())..)
            .unwrap_or_default(),
        None => text,
    };
    let event = serde_json::from_str::<Value>(payload).map_err(|source| {
        ResponsesChatStreamError::Json {
            message: text::json_error_message(payload, &source),
        }
    })?;
    Ok(DecodedEventLine::Event(event))
}
