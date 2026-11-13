use thiserror::Error;

use super::Headers;

// LiteLLM's get_async_streaming_response_iterator raises NotImplementedError with exactly this text
pub const NOT_IMPLEMENTED_STREAM: &str = "Subclasses must implement this method";

// LiteLLM's BaseLLMException carries exactly a status, a message and the upstream headers
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct BaseLlmException {
    pub status_code: u16,
    pub message: String,
    pub headers: Headers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum MessagesContractError {
    #[error("{message}")]
    NotImplemented { message: &'static str },
}
