use thiserror::Error;

use crate::openai_base_config::Headers;

// LiteLLM's JinaAIError carries a status and message only, so get_error_class drops the headers
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum JinaAiError {
    #[error("Jina AI answered with status {status_code}: {message}")]
    Provider { status_code: u16, message: String },
}

#[must_use]
pub fn get_error_class(message: &str, status_code: u16, _headers: &Headers) -> JinaAiError {
    JinaAiError::Provider {
        status_code,
        message: message.to_owned(),
    }
}
