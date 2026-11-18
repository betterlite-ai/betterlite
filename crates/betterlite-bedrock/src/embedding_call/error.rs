use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::cohere_legacy_embedding::error::CohereLegacyEmbeddingError;
use thiserror::Error;

use crate::aws_endpoints::error::AwsEndpointError;
use crate::bedrock_embedding_io::error::EmbeddingIoError;
use crate::bedrock_embedding_io::transport::TransferError;
use crate::marengo_embeddings::error::MarengoEmbeddingsError;
use crate::marengo_three::error::Marengo3Error;
use crate::nova_embeddings::error::NovaEmbeddingsError;
use crate::signing::error::SigningError;
use crate::titan_embeddings::error::TitanEmbeddingsError;

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
    #[error("the Cohere embeddings request timed out")]
    CohereTimeout {
        #[source]
        source: TransferError,
    },
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
    Titan(#[from] TitanEmbeddingsError),
    #[error(transparent)]
    Nova(#[from] NovaEmbeddingsError),
    #[error(transparent)]
    Marengo(#[from] MarengoEmbeddingsError),
    #[error(transparent)]
    Logging(#[from] PrePostError),
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
            Self::CohereTimeout { .. } => Some(408),
            Self::Marengo(MarengoEmbeddingsError::Marengo3(Marengo3Error::Provider {
                status,
                ..
            })) => Some(*status),
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
