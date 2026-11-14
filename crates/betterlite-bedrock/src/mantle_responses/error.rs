use betterlite_openai::response_input::error::ResponseInputError;
use betterlite_openai::response_params::error::ResponseParamsError;

use crate::bedrock_settings::{BedrockError, ErrorHeaders, ErrorInput, HeaderError};
use crate::mantle_inputs::error::MantleInputError;

#[derive(Debug, thiserror::Error)]
pub enum MantleResponsesError {
    #[error("{message}")]
    UnsupportedParams { message: String },
    #[error("{message}")]
    InvalidParams { message: String },
    #[error(transparent)]
    Region(#[from] MantleInputError),
    #[error(transparent)]
    Params(#[from] ResponseParamsError),
    #[error(transparent)]
    Input(#[from] ResponseInputError),
    #[error("the Bedrock Mantle Responses body could not be encoded")]
    Encode(#[source] serde_json::Error),
}

impl MantleResponsesError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::UnsupportedParams { .. } | Self::InvalidParams { .. } => Some(400),
            Self::Params(error) => Some(error.status_code()),
            Self::Input(error) => error.http_status(),
            Self::Region(_) | Self::Encode(_) => None,
        }
    }
}

pub fn error_from_reply(
    status_code: u16,
    message: &str,
    headers: &[(String, String)],
) -> Result<BedrockError, HeaderError> {
    BedrockError::try_from(ErrorInput {
        status_code,
        message: message.to_owned(),
        request: None,
        body: None,
        status_code_is_synthesized: false,
        headers: Some(ErrorHeaders::Http(headers.to_vec())),
        response: None,
    })
}
