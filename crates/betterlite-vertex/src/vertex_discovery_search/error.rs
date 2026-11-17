use std::collections::BTreeMap;

use betterlite_http::error::HttpError;
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
pub enum VertexDiscoverySearchError {
    #[error("the vector store parameter {0} must be {1}")]
    ParamType(&'static str, &'static str),
    #[error("the {0} authorization header value is not a string")]
    HeaderText(String),
    #[error(
        "Vertex AI Search extra_body may not set target-selecting fields {fields:?}: the data store is scoped by vector_store_id / vertex_engine_id and cannot be overridden per request."
    )]
    TargetSelecting { fields: Vec<String> },
    #[error(
        "Unsupported Vertex AI Search extra_body fields {fields:?} for {mode} mode. Supported fields: {supported:?}."
    )]
    UnsupportedFields {
        mode: &'static str,
        fields: Vec<String>,
        supported: Vec<String>,
    },
    #[error("vector_store_id is required when vertex_engine_id is not set")]
    StoreRequired,
    #[error("the {0} cannot be used as a URL path segment")]
    Segment(&'static str, #[source] HttpError),
    #[error(transparent)]
    Url(#[from] VertexUrlError),
    #[error("the vertex access token could not be resolved: {0}")]
    Credentials(#[source] CredentialCacheError),
    #[error("the litellm_params do not fit the Vertex AI Search target: {0}")]
    Options(#[source] serde_json::Error),
    #[error("Vertex AI Search vector store creation is not supported")]
    CreateUnsupported,
    #[error(transparent)]
    Provider(#[from] ProviderError),
}

impl VertexDiscoverySearchError {
    // LiteLLM raises BadRequestError (400) for extra_body validation and NotImplementedError (500) for creation
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::TargetSelecting { .. } | Self::UnsupportedFields { .. } => Some(400),
            Self::CreateUnsupported => Some(500),
            Self::ParamType(..)
            | Self::HeaderText(_)
            | Self::StoreRequired
            | Self::Segment(..)
            | Self::Url(_)
            | Self::Credentials(_)
            | Self::Options(_)
            | Self::Provider(_) => None,
        }
    }
}
