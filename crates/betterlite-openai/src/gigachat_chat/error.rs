use thiserror::Error;

use crate::openai_base_config::Headers;

// LiteLLM's GigaChatError is a BaseLLMException: a status, a message and optional headers
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct GigaChatError {
    pub status_code: u16,
    pub message: String,
    pub headers: Option<Headers>,
}

// The token failure LiteLLM raises as GigaChatAuthError, the same class without headers
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct GigachatTokenError {
    pub status_code: u16,
    pub message: String,
}

impl From<GigachatTokenError> for GigaChatError {
    fn from(error: GigachatTokenError) -> Self {
        Self {
            status_code: error.status_code,
            message: error.message,
            headers: None,
        }
    }
}
