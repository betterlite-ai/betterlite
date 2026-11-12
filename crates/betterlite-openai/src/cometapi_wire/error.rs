use crate::openai_base_config::Headers;
use thiserror::Error;
// LiteLLM's CometAPIException is the common native error: a message, a status and the response headers
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct CometApiError {
    pub message: String,
    pub status_code: Option<u16>,
    pub headers: Headers,
}
// both get_error_class methods build the native error from the status and headers they are given
#[must_use]
pub fn get_error_class(message: &str, status_code: u16, headers: &Headers) -> CometApiError {
    CometApiError {
        message: message.to_owned(),
        status_code: Some(status_code),
        headers: headers.clone(),
    }
}
