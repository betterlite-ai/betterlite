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
