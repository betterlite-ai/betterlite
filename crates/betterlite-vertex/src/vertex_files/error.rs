use std::num::ParseIntError;
use std::str::Utf8Error;

use betterlite_prompts::media_metadata::error::MediaMetadataError;
use thiserror::Error;

use crate::provider_metadata::timestamp::VertexTimestampError;
use crate::token_cache::error::CredentialCacheError;
use crate::vertex_batch_upload::line::LineError;
use crate::vertex_gcs_access::error::GcsAccessError;

#[derive(Debug, Error)]
pub enum VertexFilesError {
    #[error("api_key is required")]
    MissingApiKey,
    #[error(transparent)]
    Credentials(#[from] CredentialCacheError),
    #[error(transparent)]
    Media(#[from] MediaMetadataError),
    #[error(transparent)]
    Line(#[from] LineError),
    #[error("the batch file is not UTF-8")]
    ContentUtf8(#[source] Utf8Error),
    #[error("a batch line body is not a JSON object")]
    BatchBody,
    #[error("a batch line body model is not a string")]
    BatchModel,
    #[error(
        "Native Vertex batch passthrough uploads need the deployment model to name the GCS object, since native rows carry no model: pass `target_model_names` (proxy) or `model` (SDK)."
    )]
    PassthroughModel,
    #[error(
        "Vertex AI batch prediction is not supported for `custom_endpoint` deployments. The OpenAI-compatible custom endpoint path has no batch surface in LiteLLM; remove this deployment from the batch request (e.g. `target_model_names`) or use a publisher model / fine-tuned Gemini endpoint instead."
    )]
    CustomEndpointBatch,
    #[error(transparent)]
    Gcs(#[from] GcsAccessError),
    #[error("the file reply is {size} bytes, over the {limit} byte limit")]
    ResponseTooLarge { size: usize, limit: usize },
    #[error("the file reply is not a Google Cloud Storage object record")]
    Record(#[source] serde_path_to_error::Error<serde_json::Error>),
    #[error("the file reply is not valid JSON")]
    Decode(#[source] serde_json::Error),
    #[error("the file reply size is not an integer")]
    Size {
        #[source]
        source: ParseIntError,
    },
    #[error(transparent)]
    Timestamp(#[from] VertexTimestampError),
    #[error("VertexAIFilesConfig does not support file listing")]
    Listing,
}

impl VertexFilesError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::PassthroughModel | Self::CustomEndpointBatch => Some(400),
            _ => None,
        }
    }
}

mod tests;
