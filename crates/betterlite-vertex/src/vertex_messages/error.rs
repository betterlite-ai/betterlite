use thiserror::Error;

use super::beta::BetaError;
use crate::endpoint_target::error::EndpointTargetError;
use crate::token_cache::error::CredentialCacheError;

#[derive(Debug, Error)]
pub enum VertexMessagesError {
    #[error("the vertex credential could not be resolved: {source}")]
    Credentials {
        #[source]
        source: CredentialCacheError,
    },
    #[error(transparent)]
    Endpoint(#[from] EndpointTargetError),
    #[error(transparent)]
    Beta(#[from] BetaError),
    #[error("api_base is required. Unable to determine the correct api_base for the request.")]
    MissingApiBase,
}

impl VertexMessagesError {
    // LiteLLM raises a bare ValueError for a missing api_base and its credential errors escape the router as 500s.
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::Beta(error) => error.status_code(),
            Self::Credentials { .. } | Self::Endpoint(_) | Self::MissingApiBase => Some(500),
        }
    }
}
