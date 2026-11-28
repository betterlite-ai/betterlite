use std::error::Error as StdError;

use betterlite_bedrock::embedding_call::error::EmbeddingCallError;
use betterlite_bedrock::embedding_jobs::error::EmbeddingJobsError;
use betterlite_bedrock::rerank::error::RerankCallError;
use betterlite_bedrock::transcription::binding::BindingError;
use betterlite_bedrock::transcription::error::TranscriptionError;
use thiserror::Error;

use crate::bedrock_image_dispatch::error::BedrockImageError;

// Every family's own failure travels through unchanged; the binding's error is the one the caller names
#[derive(Debug, Error)]
pub enum BedrockOtherError<E>
where
    E: StdError + 'static,
{
    #[error(transparent)]
    Embedding(#[from] EmbeddingCallError),
    #[error(transparent)]
    EmbeddingStatus(#[from] EmbeddingJobsError),
    #[error(transparent)]
    Rerank(#[from] RerankCallError),
    #[error(transparent)]
    Transcription(#[from] TranscriptionError),
    #[error(transparent)]
    Binding(#[from] BindingError<E>),
    #[error(transparent)]
    Image(#[from] BedrockImageError),
    #[error("the bedrock non-chat composition runs only its own operations")]
    UnsupportedOperation,
    #[error(
        "betterlite does not run the {operation} operation through the Bedrock non-chat provider"
    )]
    Unsupported { operation: &'static str },
    #[error("the image dispatch answered with another operation result")]
    UnexpectedVariant,
}

impl<E> BedrockOtherError<E>
where
    E: StdError + 'static,
{
    // The status the family's wrapped error carries; None is what a plain Python error reports
    #[must_use]
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Embedding(error) => error.status(),
            Self::EmbeddingStatus(EmbeddingJobsError::HttpFailure { status, .. }) => Some(*status),
            Self::EmbeddingStatus(_) => None,
            Self::Rerank(error) => error.status(),
            Self::Transcription(_) | Self::Binding(_) => None,
            Self::Image(error) => error.status(),
            Self::UnsupportedOperation | Self::Unsupported { .. } | Self::UnexpectedVariant => None,
        }
    }
}
