use indexmap::IndexMap;
use thiserror::Error;

// LiteLLM's BaseLLMException: the status, message and headers a provider error carries
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct BaseLlmException {
    pub status_code: u16,
    pub message: String,
    pub headers: IndexMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SpeechContractError {
    #[error("api_base is required")]
    ApiBaseRequired,
}
