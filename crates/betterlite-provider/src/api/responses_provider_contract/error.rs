use thiserror::Error;

use super::config::Headers;

// LiteLLM's BaseLLMException carries exactly a status, a message and the upstream headers
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct BaseLlmException {
    pub status_code: u16,
    pub message: String,
    pub headers: Headers,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ResponsesContractError {
    #[error("api_base is required")]
    ApiBaseRequired,
    #[error(transparent)]
    BaseLlm(#[from] BaseLlmException),
}
