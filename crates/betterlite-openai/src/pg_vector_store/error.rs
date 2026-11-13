use betterlite_http::error::HttpError;
use thiserror::Error;

use crate::vector_stores::error::VectorStoreError;

#[derive(Debug, Error)]
pub enum PgVectorError {
    #[error(
        "PG Vector API key is required. Set PG_VECTOR_API_KEY environment variable or pass api_key in litellm_params."
    )]
    MissingApiKey,
    #[error(
        "PG Vector API base URL is required. Set PG_VECTOR_API_BASE environment variable or pass api_base in litellm_params."
    )]
    MissingApiBase,
    #[error("the vector store id cannot be used as a URL path segment")]
    Segment(#[from] HttpError),
    #[error("the OpenAI search request could not be built")]
    OpenAi {
        #[source]
        source: VectorStoreError,
    },
}
