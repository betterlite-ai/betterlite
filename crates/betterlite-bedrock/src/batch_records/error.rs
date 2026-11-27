use std::str::Utf8Error;

use betterlite_context::error::ContextError;
use betterlite_prompts::media_metadata::error::MediaMetadataError;
use betterlite_prompts::message_content::ContentError;
use thiserror::Error;

use super::families::Failure;
use crate::batch_embeddings::error::BatchEmbeddingError;
use crate::s3_access::S3AccessError;
use crate::s3_signing::S3SigningError;

#[derive(Debug, Error)]
pub enum BatchRecordsError {
    #[error("the batch file is not valid UTF-8")]
    Utf8(#[from] Utf8Error),
    #[error("a batch record is not valid JSON")]
    Json(#[source] serde_json::Error),
    #[error("a batch record must be an object whose body is an object")]
    RecordShape,
    #[error("Batch record for /v1/completions is missing required `prompt` field: model={model}")]
    MissingPrompt { model: String },
    #[error("Embedding batch records do not have a chat-completion equivalent")]
    EmbeddingChatBody,
    #[error(transparent)]
    Prompt(#[from] ContentError),
    #[error(transparent)]
    Developer(#[from] ContextError),
    #[error(transparent)]
    Media(#[from] MediaMetadataError),
    #[error(transparent)]
    Embedding(#[from] BatchEmbeddingError),
    #[error(transparent)]
    Access(#[from] S3AccessError),
    #[error(transparent)]
    Signing(#[from] S3SigningError),
    #[error("file is required")]
    FileRequired,
    #[error("{field}: Input should be valid")]
    Field { field: &'static str },
    // A family failure keeps its own message and source, so a caller sees which provider pass failed.
    #[error("the {family} request cannot be built: {source}")]
    Family {
        family: &'static str,
        #[source]
        source: Failure,
    },
    #[error("the batch value cannot be encoded as JSON")]
    Wire(#[source] serde_json::Error),
}

impl BatchRecordsError {
    pub fn family(family: &'static str, source: Failure) -> Self {
        Self::Family { family, source }
    }
}
