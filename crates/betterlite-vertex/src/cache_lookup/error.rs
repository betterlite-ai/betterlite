use betterlite_http::proxy::transport_error::TransportError;
use thiserror::Error;

use crate::endpoint_target::error::EndpointTargetError;

#[derive(Debug, Error)]
pub enum CacheLookupError {
    #[error(transparent)]
    Target(#[from] EndpointTargetError),
    #[error("the cached content list request failed: {source}")]
    Transport {
        #[source]
        source: TransportError,
    },
    #[error("the cached content list returned HTTP {status}: {message}")]
    Http { status: u16, message: String },
    #[error("a cached content list URL cannot be parsed: {source}")]
    Url {
        #[source]
        source: url::ParseError,
    },
    #[error("a cached content list header is not valid")]
    Header {
        #[source]
        source: http::Error,
    },
    #[error("a cached content page is larger than {limit} bytes")]
    PageTooLarge { limit: usize },
    #[error("a cached content page is not valid JSON: {source}")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
}

impl CacheLookupError {
    // LiteLLM's list raises VertexAIError with the upstream status, and every other failure reaches the proxy as a 500
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::Http { status, .. } => *status,
            Self::Target(_)
            | Self::Transport { .. }
            | Self::Url { .. }
            | Self::Header { .. }
            | Self::PageTooLarge { .. }
            | Self::Decode { .. } => 500,
        }
    }
}
