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

#[must_use]
pub fn get_error_class(message: &str, status_code: u16, headers: Option<Headers>) -> NlpCloudError {
    NlpCloudError {
        status_code,
        message: message.to_owned(),
        headers,
    }
}

#[derive(Debug, Error)]
pub enum NlpCloudConfigError {
    #[error("NLP Cloud has no {key} config setting")]
    UnknownSetting { key: String },
}
