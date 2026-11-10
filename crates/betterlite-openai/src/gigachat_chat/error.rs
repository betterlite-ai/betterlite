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

impl GigaChatError {
    // a request field GigaChat cannot take, which the project refuses with a 400 that names it
    pub(crate) fn bad_request(message: &str) -> Self {
        Self {
            status_code: 400,
            message: message.to_owned(),
            headers: None,
        }
    }

    // a value that reached betterlite in a shape it cannot read, which LiteLLM fails on too
    pub(crate) const fn server_error(message: String) -> Self {
        Self {
            status_code: 500,
            message,
            headers: None,
        }
    }
}
