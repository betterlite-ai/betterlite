use thiserror::Error;

use crate::openai_base_config::Headers;

#[derive(Debug, Error)]
#[error("{message}")]
pub struct GigaChatError {
    pub status_code: u16,
    pub message: String,
    pub headers: Option<Headers>,
    #[source]
    pub source: Option<serde_json::Error>,
}
