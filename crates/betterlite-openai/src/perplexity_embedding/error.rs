use thiserror::Error;

use crate::error::ProviderError;

#[derive(Debug, Error)]
pub enum PerplexityEmbeddingError {
    // LiteLLM's PerplexityEmbeddingError for a reply that is not JSON: the upstream status and the body text
    #[error("{}", .error.message)]
    Provider { error: Box<ProviderError> },
    #[error("the Perplexity embedding reply is larger than {limit} bytes")]
    TooLarge { limit: usize },
    #[error("the Perplexity embedding reply does not have the shape of an embeddings response")]
    Shape {
        #[source]
        source: serde_json::Error,
    },
    #[error("a Perplexity embedding is not valid base64: {source}")]
    Base64 {
        #[source]
        source: base64::DecodeError,
    },
    #[error("the embedding input could not be written as JSON")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}

impl PerplexityEmbeddingError {
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::Provider { error } => error.status_code,
            Self::TooLarge { .. }
            | Self::Shape { .. }
            | Self::Base64 { .. }
            | Self::Encode { .. } => 500,
        }
    }
}
