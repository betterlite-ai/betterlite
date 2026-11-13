use thiserror::Error;

use crate::error::{ErrorRequest, ErrorResponse, ProviderError};
use crate::openai_base_config::Headers;

const REQUEST_METHOD: &str = "POST";
const REQUEST_URL: &str = "https://docs.litellm.ai/docs";

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct SambaNovaError {
    pub status_code: u16,
    pub message: String,
    pub headers: Headers,
}

impl From<SambaNovaError> for ProviderError {
    fn from(error: SambaNovaError) -> Self {
        let request = ErrorRequest {
            method: REQUEST_METHOD.to_owned(),
            url: REQUEST_URL.to_owned(),
        };
        Self {
            status_code: error.status_code,
            status_code_is_synthesized: false,
            message: error.message,
            headers: Some(error.headers.into_iter().collect()),
            response: ErrorResponse {
                status_code: error.status_code,
                request: request.clone(),
            },
            request,
            body: None,
        }
    }
}

#[derive(Debug, Error)]
pub enum SambaNovaEmbeddingError {
    #[error("api_base is required for SambaNova embeddings")]
    MissingApiBase,
    #[error(transparent)]
    Reply(#[from] SambaNovaError),
    #[error("the SambaNova answer is not an embeddings reply: {source}")]
    Shape {
        #[source]
        source: serde_json::Error,
    },
}

// LiteLLM's SambaNovaError keeps the message, status and headers it is given
#[must_use]
pub fn get_error_class(message: &str, status_code: u16, headers: &Headers) -> SambaNovaError {
    SambaNovaError {
        status_code,
        message: message.to_owned(),
        headers: headers.clone(),
    }
}
