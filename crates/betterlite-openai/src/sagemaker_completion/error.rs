use thiserror::Error;

use crate::openai_base_config::Headers;

// LiteLLM's SagemakerError is a BaseLLMException: a status, a message and optional headers
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct SagemakerError {
    pub status_code: u16,
    pub message: String,
    pub headers: Option<Headers>,
}
