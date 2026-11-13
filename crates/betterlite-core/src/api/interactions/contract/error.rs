use thiserror::Error;

use super::records::ResponseHeaders;

// LiteLLM's BaseLLMException carries exactly a status, a message and the upstream headers
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct BaseLlmException {
    pub status_code: u16,
    pub message: String,
    pub headers: ResponseHeaders,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum InteractionsContractError {
    #[error(transparent)]
    BaseLlm(#[from] BaseLlmException),
}
