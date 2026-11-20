use betterlite_core::api::api_boundaries::{
    ApiBoundariesInput, ApiLimitError, ApiLimits, JsonBoundary,
};
use betterlite_models::decisions::Json;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AssistantBodyError {
    #[error(transparent)]
    Limits(#[from] ApiLimitError),
    #[error("the assistant body is not JSON: {0}")]
    Invalid(#[source] serde_json::Error),
}

// orjson.loads at proxy_server.py#L13162-L13164: one strict parse, no repair and no object coercion
pub fn decode_assistant_body(bytes: &[u8], limits: &ApiLimits) -> Result<Json, AssistantBodyError> {
    limits.check(ApiBoundariesInput::Json(JsonBoundary {
        known_bytes: bytes.len(),
    }))?;
    serde_json::from_slice(bytes).map_err(AssistantBodyError::Invalid)
}
