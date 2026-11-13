use betterlite_cache::cache_key::error::CacheKeyError;
use betterlite_http::proxy::transport_error::TransportError;
use thiserror::Error;

use crate::cache_lookup::error::CacheLookupError;
use crate::gemini_history::error::HistoryError;

#[derive(Debug, Error)]
pub enum CacheCreateError {
    #[error("the cached content endpoint request failed: {source}")]
    Endpoint {
        #[source]
        source: CacheLookupError,
    },
    #[error("the cache key could not be derived: {source}")]
    Key {
        #[source]
        source: CacheKeyError,
    },
    #[error("the cached content request could not be encoded: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the cached content body could not be built: {source}")]
    History {
        #[source]
        source: HistoryError,
    },
    #[error("the cached content create request failed: {source}")]
    Transport {
        #[source]
        source: TransportError,
    },
    #[error("the cached content create request ran past its deadline: {source}")]
    Timeout {
        #[source]
        source: TransportError,
    },
    #[error("the cached content create request returned HTTP {status}: {message}")]
    Http { status: u16, message: String },
    #[error("the cached content create response is larger than {limit} bytes")]
    TooLarge { limit: usize },
    #[error("the cached content create response at {path} is not valid: {source}")]
    Decode {
        path: String,
        #[source]
        source: serde_json::Error,
    },
}

impl CacheCreateError {
    // LiteLLM turns a create timeout into 408 and keeps an upstream status; every other failure reaches the proxy as a 500
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::Endpoint { source } => source.status_code(),
            Self::Http { status, .. } => *status,
            Self::Timeout { .. } => 408,
            Self::Key { .. }
            | Self::Encode { .. }
            | Self::History { .. }
            | Self::Transport { .. }
            | Self::TooLarge { .. }
            | Self::Decode { .. } => 500,
        }
    }

    pub(crate) const fn from_transport(source: TransportError) -> Self {
        if matches!(
            &source,
            TransportError::Deadline(_) | TransportError::ConnectTimeout(_)
        ) {
            Self::Timeout { source }
        } else {
            Self::Transport { source }
        }
    }
}
