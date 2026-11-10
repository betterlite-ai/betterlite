use betterlite_http::proxy::transport_error::TransportError;
use http::header::{InvalidHeaderName, InvalidHeaderValue};
use thiserror::Error;

use crate::cohere_embedding::CohereEmbeddingError;
use crate::error::ProviderError;

#[derive(Debug, Error)]
pub enum CohereLegacyEmbeddingError {
    #[error("the Cohere embeddings request could not be sent")]
    Transport(#[from] TransportError),
    #[error("the Cohere embeddings endpoint is not a URL")]
    Url(#[from] url::ParseError),
    #[error("the {0} header name cannot be carried in a request")]
    HeaderName(String, #[source] InvalidHeaderName),
    #[error("the {0} header value cannot be carried in a request")]
    HeaderValue(String, #[source] InvalidHeaderValue),
    #[error("the Cohere embedding JSON could not be read or written")]
    Json(#[from] serde_json::Error),
    #[error("the Cohere embedding reply carries no usable embeddings")]
    Embeddings,
    #[error("the Cohere billed units could not be read")]
    Usage(#[from] CohereEmbeddingError),
    #[error("the Cohere embeddings request failed with status {}", .0.status_code)]
    Status(Box<ProviderError>),
}
