use crate::aws_endpoints::error::AwsEndpointError;
use crate::bedrock_settings::{BedrockError, ErrorHeaders, ErrorInput, HeaderError};
use crate::signing::error::SigningError;

#[derive(Debug, thiserror::Error)]
pub enum PlatformInputError {
    #[error(
        "Missing AWS region for Claude Platform on AWS. Pass `aws_region_name` or set a standard AWS region environment value."
    )]
    MissingRegion,
    #[error("the {field} value must be a string")]
    ValueType { field: &'static str },
    #[error(transparent)]
    Region(#[from] AwsEndpointError),
    #[error(transparent)]
    Signing(#[from] SigningError),
}

impl PlatformInputError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::ValueType { .. } => Some(400),
            Self::MissingRegion => Some(401),
            Self::Region(_) | Self::Signing(_) => None,
        }
    }
}

// get_error_class hands the caller's status, message and headers to LiteLLM's BedrockError
pub fn error_class(
    error_message: String,
    status_code: u16,
    headers: Option<ErrorHeaders>,
) -> Result<BedrockError, HeaderError> {
    let input = ErrorInput {
        status_code,
        message: error_message,
        request: None,
        body: None,
        status_code_is_synthesized: false,
        headers,
        response: None,
    };
    BedrockError::try_from(input)
}
