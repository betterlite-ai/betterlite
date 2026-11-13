use thiserror::Error;

use crate::openai_base_config::Headers;

// LiteLLM's InfinityError builds this request, and its response has the error's status
pub const SYNTHETIC_REQUEST_URL: &str = "https://github.com/michaelfeil/infinity";

#[derive(Debug, Error)]
pub enum InfinityEmbeddingError {
    #[error("api_base is required for Infinity embeddings")]
    MissingApiBase,
    #[error("Infinity answered with status {status_code}: {message}")]
    Provider {
        status_code: u16,
        message: String,
        headers: Headers,
    },
    #[error("Infinity answered with status {status_code} and a body that is not JSON: {message}")]
    Json {
        status_code: u16,
        message: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("the Infinity answer is not an embeddings reply: {source}")]
    Shape {
        #[source]
        source: serde_json::Error,
    },
}

#[must_use]
pub fn get_error_class(
    message: &str,
    status_code: u16,
    headers: &Headers,
) -> InfinityEmbeddingError {
    InfinityEmbeddingError::Provider {
        status_code,
        message: message.to_owned(),
        headers: headers.clone(),
    }
}
