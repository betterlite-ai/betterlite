use thiserror::Error;

use super::EmbeddingFailure;
use crate::semantic_embedding_inputs::error::EmbeddingInputError;

#[derive(Debug, Error)]
pub enum EmbeddingReplyError {
    #[error("the embedding response carries no vector")]
    NoVector,
}

#[derive(Debug, Error)]
pub enum SemanticEmbeddingError<E>
where
    E: EmbeddingFailure,
{
    #[error("the embedding prompt could not be prepared")]
    Input(#[source] EmbeddingInputError),
    #[error("the embedding call failed")]
    Call(#[source] E),
    #[error("the embedding response could not be read")]
    Reply(#[source] EmbeddingReplyError),
    #[error("the embedding call did not finish within {seconds} seconds")]
    Timeout { seconds: f64 },
    #[error("the embedding call failed")]
    Failed {
        #[source]
        source: Box<Self>,
    },
}

impl<E> SemanticEmbeddingError<E>
where
    E: EmbeddingFailure,
{
    // LiteLLM's Redis async wrapper raises a fresh ValueError, which carries no status of its own
    #[must_use]
    pub fn status_code(&self) -> Option<u16> {
        match self {
            Self::Call(source) => source.status_code(),
            Self::Input(_) | Self::Reply(_) | Self::Timeout { .. } | Self::Failed { .. } => None,
        }
    }
}
