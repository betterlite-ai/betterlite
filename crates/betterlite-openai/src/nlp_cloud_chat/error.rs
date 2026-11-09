use thiserror::Error;

use crate::openai_base_config::Headers;

// LiteLLM's NLPCloudError is a BaseLLMException: a status, a message and optional headers
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct NlpCloudError {
    pub status_code: u16,
    pub message: String,
    pub headers: Option<Headers>,
}

#[derive(Debug, Error)]
pub enum NlpCloudConfigError {
    #[error("NLP Cloud has no {key} config setting")]
    UnknownSetting { key: String },
}
