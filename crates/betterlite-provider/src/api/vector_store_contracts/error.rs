use std::collections::BTreeMap;
use std::time::TryFromFloatSecsError;

use betterlite_models::decisions::Json;
use betterlite_openai::chat::deployment::DeploymentError;
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
    #[error("configuration field {field} must be a {expected}")]
    FieldType {
        field: &'static str,
        expected: &'static str,
    },
    #[error("configuration timeout of {seconds} seconds is not a wait this process can make")]
    Timeout {
        seconds: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error("the embedding model has no usable deployment settings: {source}")]
    Deployment {
        #[source]
        source: DeploymentError,
    },
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
