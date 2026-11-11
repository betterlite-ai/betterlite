use crate::bedrock_settings::{BedrockError, ErrorHeaders, ErrorInput, HeaderError};

// Both get_error_class methods hand the upstream status, message and headers to the Bedrock error unchanged
pub fn error_class(
    message: String,
    status_code: u16,
    headers: ErrorHeaders,
) -> Result<BedrockError, HeaderError> {
    BedrockError::try_from(ErrorInput {
        status_code,
        message,
        request: None,
        body: None,
        status_code_is_synthesized: false,
        headers: Some(headers),
        response: None,
    })
}
