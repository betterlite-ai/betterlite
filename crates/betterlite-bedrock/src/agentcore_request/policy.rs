use crate::bedrock_settings::{BedrockError, ErrorHeaders, ErrorInput, HeaderError};

// AgentCore maps no OpenAI parameter and streams through its custom wrapper without a body stream flag
pub const SUPPORTED_OPENAI_PARAMS: [&str; 0] = [];
pub const HAS_CUSTOM_STREAM_WRAPPER: bool = true;
pub const SUPPORTS_STREAM_PARAM_IN_BODY: bool = false;
pub const SHOULD_FAKE_STREAM: bool = false;

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
