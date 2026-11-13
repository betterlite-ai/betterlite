use betterlite_http::proxy::transport_error::TransportError;
use thiserror::Error;

use crate::cache_lookup::error::CacheLookupError;
use crate::endpoint_target::error::EndpointTargetError;
use crate::model_urls::error::VertexUrlError;
use crate::token_cache::error::CredentialCacheError;
use crate::vertex_fine_tuning::records::error::TuningRecordError;

#[derive(Debug, Error)]
pub enum TuningExecutionError {
    #[error("the vertex credentials could not be resolved: {source}")]
    Credentials {
        #[source]
        source: CredentialCacheError,
    },
    #[error("the tuning job request could not be prepared: {source}")]
    Target {
        #[source]
        source: EndpointTargetError,
    },
    #[error("the tuning job URL cannot be built: {source}")]
    VertexUrl(#[from] VertexUrlError),
    #[error("the tuning job URL cannot be parsed: {source}")]
    Url {
        #[source]
        source: url::ParseError,
    },
    #[error("the tuning job request body cannot be encoded: {source}")]
    Encoded {
        #[source]
        source: serde_json::Error,
    },
    #[error("a tuning job request header is not valid: {source}")]
    Header {
        #[source]
        source: CacheLookupError,
    },
    #[error(transparent)]
    Record(#[from] TuningRecordError),
    #[error("the tuning job request failed: {source}")]
    Transport {
        #[source]
        source: TransportError,
    },
    #[error("creating the tuning job failed with status {status}: {body}")]
    Http { status: u16, body: String },
}

impl TuningExecutionError {
    // LiteLLM keeps the upstream status on a non-200 answer and fails every other step with a 500
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::Http { status, .. } => *status,
            Self::Credentials { .. }
            | Self::Target { .. }
            | Self::VertexUrl(_)
            | Self::Url { .. }
            | Self::Encoded { .. }
            | Self::Header { .. }
            | Self::Record(_)
            | Self::Transport { .. } => 500,
        }
    }
}
