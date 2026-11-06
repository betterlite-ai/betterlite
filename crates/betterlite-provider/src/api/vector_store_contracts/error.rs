use std::collections::BTreeMap;
use std::time::TryFromFloatSecsError;

use betterlite_models::decisions::Json;
use betterlite_openai::embedding::EmbeddingError;
use thiserror::Error;

pub type HeaderMap = BTreeMap<String, Json>;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct BaseLlmException {
    pub status_code: u16,
    pub message: String,
    pub headers: HeaderMap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum VectorStoreContractError {
    #[error("api_base is required")]
    ApiBaseRequired,
}

#[derive(Debug, Error)]
pub enum VectorStoreEmbeddingError {
    #[error(
        "{name} is passed explicitly, so setting it in the configuration is a duplicate argument"
    )]
    DuplicateArgument { name: String },
    #[error("{name} is not an embedding configuration parameter betterlite supports")]
    UnsupportedParameter { name: String },
    #[error("configuration field {field} must be a {expected}")]
    FieldType {
        field: &'static str,
        expected: &'static str,
    },
    #[error("configuration field {field} is not a valid {expected}")]
    FieldValue {
        field: &'static str,
        expected: &'static str,
    },
    #[error("configuration timeout of {seconds} seconds is not a wait this process can make")]
    Timeout {
        seconds: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error("no deployment serves the embedding model {model}")]
    UnknownModel { model: String },
    #[error("the embedding request could not be built: {source}")]
    Request {
        #[source]
        source: EmbeddingError,
    },
    #[error("the embedding request failed: {source}")]
    Embedding {
        #[source]
        source: EmbeddingError,
    },
    #[error("the router refused the embedding call: {source}")]
    Router {
        #[source]
        source: BaseLlmException,
    },
}
