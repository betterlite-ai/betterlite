use thiserror::Error;

use super::config::Headers;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct BaseLlmException {
    pub status_code: u16,
    pub message: String,
    pub headers: Headers,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FileContractError {
    #[error("api_base is required")]
    ApiBaseRequired,
    #[error("the file response body is invalid: {0}")]
    Body(String),
    #[error(transparent)]
    Upstream(#[from] BaseLlmException),
}
