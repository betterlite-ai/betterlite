use crate::openai_base_config::Headers;
use thiserror::Error;

// LiteLLM's ScalewayAudioTranscriptionException is a BaseLLMException: a message, a status and the response headers
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct ScalewayAudioError {
    pub message: String,
    pub status_code: u16,
    pub headers: Headers,
}

pub const MISSING_KEY_MESSAGE: &str = "Scaleway API key not found. Pass `api_key=...` or set the SCW_SECRET_KEY environment variable.";

#[must_use]
pub fn get_error_class(message: &str, status_code: u16, headers: &Headers) -> ScalewayAudioError {
    ScalewayAudioError {
        message: message.to_owned(),
        status_code,
        headers: headers.clone(),
    }
}

// Scaleway rejects a missing key before it builds any header, so this error carries no headers of its own
#[must_use]
pub fn missing_key() -> ScalewayAudioError {
    ScalewayAudioError {
        message: String::from(MISSING_KEY_MESSAGE),
        status_code: 401,
        headers: Headers::new(),
    }
}
