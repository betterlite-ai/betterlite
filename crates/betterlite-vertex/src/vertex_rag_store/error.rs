use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use thiserror::Error;

use crate::model_urls::error::VertexUrlError;
use crate::token_cache::error::CredentialCacheError;

// B1058's BaseLlmException shape, so the executor converts a decode failure without losing status or headers
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct ProviderError {
    pub status_code: u16,
    pub message: String,
    pub headers: BTreeMap<String, Json>,
}

#[derive(Debug, Error)]
pub enum VertexRagStoreError {
    #[error("the vector store parameter {field} must be {expected}")]
    ParamType {
        field: &'static str,
        expected: &'static str,
    },
    #[error("the {name} authorization header value is not a string")]
    HeaderText { name: String },
    #[error("the {name} authorization header value is not JSON text: {source}")]
    HeaderJson {
        name: String,
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Url(#[from] VertexUrlError),
    #[error("the vertex access token could not be resolved: {source}")]
    Credentials {
        #[source]
        source: CredentialCacheError,
    },
    #[error(transparent)]
    Provider(#[from] ProviderError),
}
