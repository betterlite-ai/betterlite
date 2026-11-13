use crate::openai_base_config::Headers;
use thiserror::Error;

// LiteLLM's VercelAIGatewayException adds nothing to the base native error
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct VercelAiGatewayError {
    pub message: String,
    pub status_code: u16,
    pub headers: Headers,
}

#[must_use]
pub fn get_error_class(message: &str, status_code: u16, headers: &Headers) -> VercelAiGatewayError {
    VercelAiGatewayError {
        message: message.to_owned(),
        status_code,
        headers: headers.clone(),
    }
}
