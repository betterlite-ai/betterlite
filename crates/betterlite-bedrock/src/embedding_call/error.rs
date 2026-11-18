use betterlite_openai::cohere_legacy_embedding::error::CohereLegacyEmbeddingError;
use thiserror::Error;

use crate::aws_endpoints::error::AwsEndpointError;
use crate::bedrock_embedding_io::error::EmbeddingIoError;
use crate::signing::error::SigningError;

#[derive(Debug, Error)]
pub enum EmbeddingCallError {
    #[error(transparent)]
    Embedding(#[from] EmbeddingIoError),
    #[error(transparent)]
    Endpoint(#[from] AwsEndpointError),
    #[error(transparent)]
    Signing(#[from] SigningError),
    #[error(transparent)]
    Cohere(#[from] CohereLegacyEmbeddingError),
    #[error("the embedding JSON cannot be read or written")]
    Json {
        #[from]
        source: serde_json::Error,
    },
    #[error("invalid {field}")]
    Field {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Titan(#[from] crate::titan_embeddings::error::TitanEmbeddingsError),
    #[error(transparent)]
    Nova(#[from] crate::nova_embeddings::error::NovaEmbeddingsError),
    #[error(transparent)]
    Marengo(#[from] crate::marengo_embeddings::error::MarengoEmbeddingsError),
    #[error(transparent)]
    Logging(#[from] betterlite_logging::logging_pre_post::error::PrePostError),
    #[error("the embedding timeout {seconds} is not a usable duration")]
    Timeout { seconds: f64 },
    #[error(
        "Unable to determine bedrock embedding provider for model: {model}. Supported providers: ['cohere', 'amazon', 'twelvelabs', 'nova']"
    )]
    Provider { model: String },
    #[error("Unable to map Bedrock request to provider")]
    Unmapped,
}

impl EmbeddingCallError {
    #[must_use]
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Embedding(error) => error.status(),
            Self::Cohere(CohereLegacyEmbeddingError::Status(error)) => Some(error.status_code),
            Self::Marengo(crate::marengo_embeddings::error::MarengoEmbeddingsError::Marengo3(
                crate::marengo_three::error::Marengo3Error::Provider { status, .. },
            )) => Some(*status),
            Self::Endpoint(_)
            | Self::Signing(_)
            | Self::Cohere(_)
            | Self::Json { .. }
            | Self::Field { .. }
            | Self::Titan(_)
            | Self::Nova(_)
            | Self::Marengo(_)
            | Self::Logging(_)
            | Self::Timeout { .. }
            | Self::Provider { .. }
            | Self::Unmapped => None,
        }
    }
}
