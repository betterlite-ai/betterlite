use thiserror::Error;

use crate::error::{ProviderError, ProviderErrorParts};
use crate::openai_base_config::Headers;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct SambaNovaError {
    pub status_code: u16,
    pub message: String,
    pub headers: Headers,
}

// BaseLLMException fills an absent request with a POST to the docs page and a response with the same status
impl From<SambaNovaError> for ProviderError {
    fn from(error: SambaNovaError) -> Self {
        Self::from(ProviderErrorParts {
            status_code: error.status_code,
            message: error.message,
            headers: Some(error.headers.into_iter().collect()),
            request: None,
            response: None,
            body: None,
            status_code_is_synthesized: false,
        })
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
