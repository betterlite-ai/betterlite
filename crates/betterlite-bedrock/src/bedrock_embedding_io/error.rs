use betterlite_secret_managers::secret_lookup::error::SecretLookupError;
use thiserror::Error;
use tokio::task::JoinError;

use super::transport::TransferError;
use crate::aws_settings::AuthError;
use crate::bedrock_settings::{BedrockError, HeaderError};
use crate::credentials::error::CredentialError;
use crate::marengo_embeddings::error::MarengoEmbeddingsError;
use crate::nova_embeddings::error::NovaEmbeddingsError;
use crate::titan_embeddings::error::TitanEmbeddingsError;

#[derive(Debug, Error)]
pub enum EmbeddingIoError {
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error("invalid aws_region_name")]
    Region {
        #[source]
        source: serde_json::Error,
    },
    #[error("the managed region secret lookup worker failed")]
    RegionWorker {
        #[source]
        source: JoinError,
    },
    #[error("the managed region secret lookup worker ended without a result")]
    RegionWorkerClosed,
    #[error(transparent)]
    Secret(#[from] SecretLookupError),
    #[error(transparent)]
    Credential(#[from] CredentialError),
    #[error(transparent)]
    Header(#[from] HeaderError),
    #[error("the embedding request body cannot be written as JSON")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the embedding reply is not a JSON object")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the embedding reply does not hold the declared shape")]
    Params {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Transport(#[from] TransferError),
    #[error("bedrock error ({status})")]
    Provider {
        status: u16,
        error: Box<BedrockError>,
    },
    #[error(transparent)]
    Titan(#[from] TitanEmbeddingsError),
    #[error(transparent)]
    Nova(#[from] NovaEmbeddingsError),
    #[error(transparent)]
    Marengo(#[from] MarengoEmbeddingsError),
    #[error("the embedding reply holds a vector no JSON value can carry")]
    Unrepresentable,
    #[error(
        "the embedding reply holds a value that is neither a number vector nor an encoded string"
    )]
    MalformedEmbedding,
    #[error("an embedding vector holds a value that is not a number")]
    NotANumber,
    #[error("the embedding reply holds more records than an index can address")]
    IndexOverflow,
    #[error("Unable to map model response to known provider format. model={model}")]
    Unmapped { model: String },
}

impl EmbeddingIoError {
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        if let Self::Provider { status, .. } = self {
            Some(*status)
        } else {
            None
        }
    }
}
