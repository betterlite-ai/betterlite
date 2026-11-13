use thiserror::Error;

use crate::openai_base_config::Headers;

// LiteLLM's DashScopeError is a BaseLLMException: the upstream status, the message and the reply headers
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct DashscopeError {
    pub status_code: u16,
    pub message: String,
    pub headers: Headers,
}

#[must_use]
pub fn get_error_class(
    message: &str,
    status_code: u16,
    headers: Option<&Headers>,
) -> DashscopeError {
    DashscopeError {
        status_code,
        message: message.to_owned(),
        headers: headers.cloned().unwrap_or_default(),
    }
}
