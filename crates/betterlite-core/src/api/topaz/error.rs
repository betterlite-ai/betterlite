use betterlite_openai::api::image_contracts::records::ImageHeaders;
use thiserror::Error;

// TopazModelInfo.validate_environment's ValueError and TopazImageVariationConfig.map_openai_params' size assertion
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TopazError {
    #[error(
        "API key is required for Topaz image variations. Set via `TOPAZ_API_KEY` or `api_key=..`"
    )]
    MissingApiKey,
    #[error("size must be in the format of widthxheight")]
    SizeFormat,
}

// get_error_class's TopazException keeps the upstream status, its message and the headers it came with
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct TopazException {
    pub status_code: u16,
    pub message: String,
    pub headers: ImageHeaders,
}

impl TopazException {
    // BaseLLMException stores the three values as it received them
    #[must_use]
    pub fn new(status_code: u16, message: String, headers: ImageHeaders) -> Self {
        Self {
            status_code,
            message,
            headers,
        }
    }
}
