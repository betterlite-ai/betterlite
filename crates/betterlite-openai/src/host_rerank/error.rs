use thiserror::Error;

use crate::api::rerank_contracts::wire::RerankContractError;
use crate::api::responses_shapes::wire::ShapeDecodeError;
use crate::cohere_rerank::CohereRerankError;

#[derive(Debug, Error)]
pub enum HostRerankError {
    #[error(
        "api_key is required. Set via `api_key` parameter or `JINA_API_KEY` environment variable."
    )]
    MissingApiKey,
    #[error("api_base is required for Infinity rerank")]
    MissingApiBase,
    #[error("the Jina api_base `{base}` is not an absolute URL with a host")]
    InvalidApiBase { base: String },
    #[error("{message}")]
    Status { status_code: u16, message: String },
    #[error("the rerank answer with status {status_code} is not JSON: {source}")]
    Json {
        status_code: u16,
        body: String,
        source: serde_json::Error,
    },
    #[error("the rerank answer cannot be decoded: {source}")]
    Decode { source: ShapeDecodeError },
    #[error("No results found in the response")]
    NoResults,
    #[error("a rerank result document must be a string or, for Jina, an object with a text string")]
    Document,
    #[error("the usage counter `{name}` is not an integer")]
    Counter { name: &'static str },
    #[error("the usage counters do not fit a 64-bit integer")]
    CounterOverflow,
    #[error("the token cost does not fit an exact decimal")]
    CostOverflow,
    #[error(transparent)]
    Contract(#[from] RerankContractError),
    #[error(transparent)]
    Request(#[from] CohereRerankError),
}
