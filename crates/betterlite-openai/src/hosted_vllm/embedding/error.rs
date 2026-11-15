use thiserror::Error;

use crate::openai_base_config::Headers;

#[derive(Debug, Error)]
pub enum HostedVllmEmbeddingError {
    #[error("api_base is required for hosted_vllm embeddings")]
    MissingApiBase,
    #[error("{message}")]
    Provider {
        message: String,
        status_code: u16,
        headers: Headers,
    },
    #[error("the provider answer is not an embeddings response: {source}")]
    Shape {
        #[source]
        source: serde_json::Error,
    },
}

impl HostedVllmEmbeddingError {
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::Provider { status_code, .. } => *status_code,
            Self::MissingApiBase | Self::Shape { .. } => 500,
        }
    }
}
