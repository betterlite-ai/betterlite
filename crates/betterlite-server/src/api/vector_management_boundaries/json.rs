use betterlite_core::api::api_boundaries::{
    ApiBoundariesInput, ApiLimitError, ApiLimits, JsonBoundary,
};
use betterlite_models::decisions::Json;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ManagementJsonError {
    #[error(transparent)]
    Limits(#[from] ApiLimitError),
    #[error("stored JSON text is not valid JSON")]
    Syntax(#[from] serde_json::Error),
}

// json.loads at the boundary: the stored bytes become the value they encode, bounded before parsing
pub fn decode_management_json(
    bytes: &[u8],
    limits: ApiLimits,
) -> Result<Json, ManagementJsonError> {
    limits.check(ApiBoundariesInput::Json(JsonBoundary {
        known_bytes: bytes.len(),
    }))?;
    serde_json::from_slice(bytes).map_err(ManagementJsonError::Syntax)
}
