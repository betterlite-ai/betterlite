use std::str::from_utf8;

use betterlite_anthropic::api::messages_errors::wire::ResponseRecord;
use betterlite_anthropic::stream_events::error::AnthropicError;
use serde_json::error::Category;
use thiserror::Error;

use crate::api::api_boundaries::{ApiBoundariesInput, ApiLimitError, ApiLimits, JsonBoundary};

// A bounded body, the AnthropicError json() raises, or the TypeError ** raises for a non-mapping.
#[derive(Debug, Error)]
pub enum ResponseJsonError {
    #[error(transparent)]
    Limit(#[from] ApiLimitError),
    #[error(transparent)]
    Anthropic(#[from] AnthropicError),
    #[error("the Messages response body is not a JSON object")]
    NotAnObject,
}

impl ResponseJsonError {
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::Limit(error) => error.fields().status,
            Self::Anthropic(error) => error.status_code,
            Self::NotAnObject => 500,
        }
    }
}

// transform_anthropic_messages_response: the object comes back as sent, so the requested model never replaces the provider's.
pub fn decode_messages_response(
    body: &[u8],
    status: u16,
    limits: &ApiLimits,
) -> Result<ResponseRecord, ResponseJsonError> {
    limits.check(ApiBoundariesInput::Json(JsonBoundary {
        known_bytes: body.len(),
    }))?;
    match serde_json::from_slice::<ResponseRecord>(body) {
        Ok(fields) => Ok(fields),
        // a valid body of the wrong shape only fails the keyword expansion, which is an internal failure
        Err(source) if source.classify() == Category::Data => Err(ResponseJsonError::NotAnObject),
        Err(_source) => Err(ResponseJsonError::Anthropic(AnthropicError {
            message: body_text(body),
            status_code: status,
        })),
    }
}

// Python's json.loads reads the body as text, and bytes that are not UTF-8 are named by their count
fn body_text(body: &[u8]) -> String {
    from_utf8(body).map_or_else(
        |_| format!("{} bytes that are not UTF-8", body.len()),
        ToOwned::to_owned,
    )
}
