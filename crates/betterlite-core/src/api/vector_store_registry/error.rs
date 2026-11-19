use thiserror::Error;

#[derive(Debug, Error)]
pub enum VectorStoreRegistryError {
    #[error("vector_store_id is required for initializing vector store, got vector_store_id=None")]
    MissingVectorStoreId,
    #[error(
        "custom_llm_provider is required for initializing vector store, got custom_llm_provider=None"
    )]
    MissingCustomLlmProvider,
    #[error("litellm_params.{field} cannot hold this JSON type")]
    ParamType { field: &'static str },
    #[error("{field} must be a list of strings")]
    IdList { field: &'static str },
    #[error("rewrite_query is not a supported tool parameter")]
    RewriteQueryNotSupported,
    #[error("tool parameter {field} cannot hold this JSON type")]
    ToolParamType { field: &'static str },
    #[error("the vector store record could not be encoded as JSON")]
    Json(#[from] serde_json::Error),
}
