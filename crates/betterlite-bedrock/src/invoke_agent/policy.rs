use crate::bedrock_settings::{BedrockError, ErrorHeaders, ErrorInput, HeaderError};

// InvokeAgent maps no OpenAI parameter and always answers through the fake-stream wrapper
pub const SUPPORTED_OPENAI_PARAMS: [&str; 0] = [];
pub const SHOULD_FAKE_STREAM: bool = true;

pub fn get_error_class(
    message: String,
    status_code: u16,
    headers: Option<ErrorHeaders>,
) -> Result<BedrockError, HeaderError> {
    BedrockError::try_from(ErrorInput {
        status_code,
        message,
        request: None,
        body: None,
        status_code_is_synthesized: false,
        headers,
        response: None,
    })
}
