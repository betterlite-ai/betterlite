use std::collections::BTreeMap;

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct BaseLlmException {
    pub status_code: u16,
    pub message: String,
    pub headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ContainerError {
    #[error("api_base is required")]
    ApiBaseRequired,
    #[error("the container response body is invalid: {0}")]
    Decode(String),
    #[error(transparent)]
    Upstream(#[from] BaseLlmException),
}
