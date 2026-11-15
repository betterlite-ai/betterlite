use crate::openai_base_config::Headers;
use thiserror::Error;

// LiteLLM's OVHCloudException is a BaseLLMException: a message, a status and the response headers
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct OvhCloudError {
    pub message: String,
    pub status_code: Option<u16>,
    pub headers: Headers,
}

#[must_use]
pub fn get_error_class(message: &str, status_code: u16, headers: &Headers) -> OvhCloudError {
    OvhCloudError {
        message: message.to_owned(),
        status_code: Some(status_code),
        headers: headers.clone(),
    }
}
