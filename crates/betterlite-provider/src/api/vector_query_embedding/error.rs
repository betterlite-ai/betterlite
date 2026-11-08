use crate::api::vector_store_contracts::error::{
    VectorStoreContractError, VectorStoreEmbeddingError,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum QueryEmbeddingError {
    #[error(
        "litellm_embedding_model is required in litellm_params for this vector store. Example: litellm_params['litellm_embedding_model'] = 'openai/text-embedding-3-small'"
    )]
    MissingModel,
    #[error("the litellm_embedding_config keys must be text, or an integer written in decimal")]
    ConfigKey,
    #[error("the embedding answer has no first record")]
    NoFirstRecord,
    #[error("the first embedding record has no embedding")]
    NoEmbedding,
    #[error("the first embedding is not a list of numbers")]
    NotAVector,
    #[error("Failed to generate embedding for query: {source}")]
    Execution {
        #[source]
        source: VectorStoreEmbeddingError,
    },
}

#[derive(Debug, Error)]
pub enum DirectStoreError {
    #[error("{message}")]
    NotImplemented { message: &'static str },
    #[error(transparent)]
    Contract(#[from] VectorStoreContractError),
}
