use std::error::Error;

use strum::Display;
use thiserror::Error as ThisError;

use crate::semantic_math::MathError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
#[strum(serialize_all = "snake_case")]
pub enum EmbeddingStage {
    Documents,
    Query,
}

#[derive(Debug, ThisError)]
pub enum RouterError<E>
where
    E: Error + Send + Sync + 'static,
{
    #[error("the {stage} embedding call failed: {source}")]
    Embedding {
        stage: EmbeddingStage,
        #[source]
        source: E,
    },
    #[error("the {stage} embedding call returned {found} vectors for {expected} inputs")]
    VectorCount {
        stage: EmbeddingStage,
        expected: usize,
        found: usize,
    },
    #[error("the {stage} embedding vectors are unusable: {source}")]
    Vectors {
        stage: EmbeddingStage,
        #[source]
        source: MathError,
    },
    // a shared build whose task ended without storing a result, as an abandoned Python build task does
    #[error("the shared route index build ended without a result")]
    BuildAborted,
}
