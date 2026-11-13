use std::collections::BTreeMap;

use thiserror::Error;

pub type ResponseHeaders = BTreeMap<String, String>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchTransform {
    Request,
    Response,
}

#[derive(Debug, Error)]
pub enum RagflowError {
    #[error("api_key is required (set RAGFLOW_API_KEY env var or pass in litellm_params)")]
    MissingAuthKey,
    #[error("RAGFLOW_API_KEY is required (set env var or pass in litellm_params)")]
    MissingEnvironmentKey,
    #[error("name is required for RAGFlow dataset creation")]
    MissingName,
    #[error(
        "chunk_method and pipeline_id are mutually exclusive. Specify either chunk_method or pipeline_id, not both."
    )]
    ChunkMethodExclusive,
    #[error("RAGFlow response missing dataset id")]
    MissingDatasetId,
    #[error("RAGFlow vector stores support dataset management only, not search/retrieval")]
    SearchUnsupported { transform: SearchTransform },
    #[error("{message}")]
    ErrorResponse {
        status: u16,
        headers: ResponseHeaders,
        message: String,
    },
    #[error("RAGFlow returned a response betterlite cannot read")]
    UnreadableResponse {
        status: u16,
        headers: ResponseHeaders,
        #[source]
        source: CreateResponseError,
    },
}

#[derive(Debug, Error)]
pub enum CreateResponseError {
    #[error("the response body is not JSON")]
    Body {
        #[source]
        source: serde_json::Error,
    },
    #[error("the response body is not a JSON object")]
    NotAnObject,
    #[error("the data field is not an object")]
    Data,
    #[error("create_time is not a number")]
    CreateTime,
    #[error("create_time is out of range")]
    CreateTimeRange,
}
