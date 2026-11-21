use std::str::{self, Utf8Error};

use betterlite_core::api::api_boundaries::{
    ApiBoundariesInput, ApiLimitError, ApiLimits, JsonBoundary,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SdpError {
    #[error(transparent)]
    Limit(#[from] ApiLimitError),
    #[error("the SDP offer is not UTF-8")]
    Utf8(#[source] Utf8Error),
}

// LiteLLM strings the offer for the multipart field and never parses SDP, so the body cap and UTF-8 are the whole check
pub fn decode_realtime_sdp(bytes: &[u8], limits: ApiLimits) -> Result<&str, SdpError> {
    let _checked = limits.check(ApiBoundariesInput::Json(JsonBoundary {
        known_bytes: bytes.len(),
    }))?;
    str::from_utf8(bytes).map_err(SdpError::Utf8)
}
