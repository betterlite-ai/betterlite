use serde_json::Value;
use thiserror::Error;

use crate::openai_base_config::Headers;
use crate::volcengine_common::VolcEngineError;

#[derive(Debug, Error)]
pub enum VolcEngineEmbeddingError {
    #[error("api_key is required for Volcengine authentication")]
    MissingApiKey,
    #[error("Unsupported encoding_format: {value}. Volcengine supports: float, base64, null")]
    UnsupportedEncodingFormat { value: Value },
    #[error("Unsupported parameter for Volcengine: {name}")]
    UnsupportedParam { name: String },
}

// LiteLLM's get_error_class wraps the reply's statuses in its own exception with the headers it is given
#[must_use]
pub fn get_error_class(message: &str, status_code: u16, headers: &Headers) -> VolcEngineError {
    VolcEngineError {
        status_code,
        message: message.to_owned(),
        headers: headers.clone(),
    }
}
