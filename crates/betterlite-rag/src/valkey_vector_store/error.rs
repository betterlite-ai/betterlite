use betterlite_provider::api::vector_store_contracts::error::{
    VectorStoreContractError, VectorStoreEmbeddingError,
};
use betterlite_redis::redis_live_client::LiveClientError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ValkeyError {
    #[error("the {field} value has the wrong type: expected {expected}")]
    FieldType {
        field: &'static str,
        expected: &'static str,
    },
    #[error("valkey_port must be between 0 and 65535, got {value}")]
    PortRange { value: i64 },
    #[error(
        "litellm_embedding_model is required in litellm_params for the Valkey vector store. \
         Example: litellm_params['litellm_embedding_model'] = 'openai/text-embedding-3-small'"
    )]
    EmbeddingModelRequired,
    #[error(
        "valkey_host is required in litellm_params for the Valkey vector store. \
         Set it on the vector store's litellm_params, e.g. valkey_host: my-valkey.example.com"
    )]
    HostRequired,
    #[error("query must not be empty")]
    EmptyQuery,
    #[error("Valkey vector store does not support the filters parameter yet")]
    FiltersUnsupported,
    #[error("max_num_results must be between 1 and 50, got {value}")]
    MaxResultsOutOfRange { value: i64 },
    #[error("Valkey vector store is search-only; create indexes with FT.CREATE directly")]
    SearchOnly,
    #[error(
        "Valkey direct vector search runs FT.SEARCH itself; there is no HTTP request or response shape"
    )]
    DirectSearch,
    #[error("api_base is required")]
    ApiBaseRequired,
    #[error("the embedding call failed: {source}")]
    Embedding {
        #[source]
        source: VectorStoreEmbeddingError,
    },
    #[error("the embedding response contained no vectors")]
    NoEmbedding,
    #[error("the search result for document '{id}' has no vector_distance field")]
    MissingDistance { id: String },
    #[error("the search result for document '{id}' has an invalid vector_distance: '{value}'")]
    InvalidDistance { id: String, value: String },
    #[error("the FT.SEARCH reply is not usable: {detail}")]
    UnexpectedReply { detail: &'static str },
    #[error(transparent)]
    Redis(#[from] LiveClientError),
}

impl From<VectorStoreContractError> for ValkeyError {
    fn from(source: VectorStoreContractError) -> Self {
        match source {
            VectorStoreContractError::ApiBaseRequired => Self::ApiBaseRequired,
        }
    }
}
