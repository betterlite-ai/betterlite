use std::collections::BTreeMap;

use thiserror::Error;

use crate::api::vector_store_records::wire::VectorStoreRecordError;

pub type ResponseHeaders = BTreeMap<String, String>;

#[derive(Debug, Error)]
pub enum MilvusError {
    #[error(
        "MILVUS_API_KEY is not set. Either set it in the litellm_params or set the MILVUS_API_KEY environment variable."
    )]
    MissingApiKey,
    #[error(
        "Milvus API base URL is required. Set MILVUS_API_BASE environment variable or pass api_base in litellm_params."
    )]
    MissingApiBase,
    #[error("the query vector value at index {index} is not a finite number")]
    InvalidVector { index: usize },
    #[error("Milvus returned a search response betterlite cannot project")]
    Provider {
        status: u16,
        headers: ResponseHeaders,
        #[source]
        source: SearchResponseError,
    },
    #[error("Milvus returned a score outside the exact decimal range")]
    ScoreOutOfRange(#[source] VectorStoreRecordError),
    #[error("Milvus vector store creation is not supported")]
    CreateUnsupported(#[source] UnsupportedCreate),
}

#[derive(Debug, Error)]
pub enum UnsupportedCreate {
    #[error("Milvus vector store create request is not supported")]
    Request { api_base: String },
    #[error("Milvus vector store create response is not supported")]
    Response {
        status: u16,
        headers: ResponseHeaders,
    },
}

#[derive(Debug, Error)]
pub enum SearchResponseError {
    #[error("the response body is not JSON")]
    Body {
        #[source]
        source: serde_json::Error,
    },
    #[error("the response body is not a JSON object")]
    NotAnObject,
    #[error("the data field is not an array of results")]
    Data,
    #[error("search result {index} is not an object")]
    Row { index: usize },
    #[error("search result {index} has a distance that is not a number")]
    Distance { index: usize },
}
