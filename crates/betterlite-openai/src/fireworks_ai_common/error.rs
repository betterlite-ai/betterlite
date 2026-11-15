use thiserror::Error;

use crate::openai_base_config::Headers;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct FireworksError {
    pub status_code: u16,
    pub message: String,
    pub headers: Option<Headers>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("FIREWORKS_API_KEY is not set")]
pub struct MissingApiKey;
