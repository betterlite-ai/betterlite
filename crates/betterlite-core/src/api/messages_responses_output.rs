use betterlite_prompts::arguments::json::Json;

use crate::api::api_boundaries::{ApiBoundariesInput, ApiLimits, JsonBoundary};

use error::MessagesResponsesOutputError;

pub mod error;
pub mod options;
pub mod outputs;
pub mod records;
pub mod request;
pub mod response;
pub mod service;

mod tests;

// RESPONSES_INCLUDE_ENCRYPTED_REASONING: reasoning items come back encrypted so the next turn can replay them
const ENCRYPTED_REASONING_INCLUDE: &str = "reasoning.encrypted_content";

// json.loads of a Responses function call's arguments, at the size the JSON boundary allows
pub fn decode_messages_responses_out(
    bytes: &[u8],
    limits: &ApiLimits,
) -> Result<Json, MessagesResponsesOutputError> {
    limits.check(ApiBoundariesInput::Json(JsonBoundary {
        known_bytes: bytes.len(),
    }))?;
    serde_json::from_slice(bytes).map_err(|source| MessagesResponsesOutputError::Json { source })
}
