use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use http::header::InvalidHeaderValue;
use thiserror::Error;

use crate::endpoint_target::error::EndpointTargetError;
use crate::token_cache::error::CredentialCacheError;
use crate::vertex_batch_create::error::VertexBatchCreateError;
use crate::vertex_batch_records::error::BatchRecordsError;

#[derive(Debug, Error)]
pub enum VertexBatchLifecycleError {
    #[error("the Vertex credentials could not be resolved: {source}")]
    Credentials {
        #[source]
        source: CredentialCacheError,
    },
    #[error("the Authorization header cannot carry the access token")]
    Header {
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the batch job identifier cannot be one path segment: {source}")]
    Segment {
        #[from]
        source: HttpError,
    },
    #[error(transparent)]
    Target(#[from] EndpointTargetError),
    #[error(transparent)]
    Batch(#[from] VertexBatchCreateError),
    #[error("the Vertex batch job projection is invalid: {source}")]
    Records {
        #[source]
        source: BatchRecordsError,
    },
    #[error("the Vertex batch URL is not valid: {source}")]
    Url {
        #[source]
        source: url::ParseError,
    },
    #[error("the Vertex batch retrieve request returned HTTP {status}: {message}")]
    RetrieveStatus { status: u16, message: String },
    #[error("the Vertex batch retrieve request failed: {source}")]
    ProtectedGet {
        #[source]
        source: VertexBatchCreateError,
    },
    #[error("the Vertex batch list request returned HTTP {status}: {message}")]
    ListStatus { status: u16, message: String },
    #[error("the Vertex batch list request failed: {source}")]
    ListTransport {
        #[source]
        source: TransportError,
    },
    #[error("the Vertex batch cancel request returned HTTP {status}: {message}")]
    CancelStatus { status: u16, message: String },
    #[error("the Vertex batch cancel request failed: {source}")]
    CancelTransport {
        #[source]
        source: TransportError,
    },
    #[error("the Vertex batch cancel request ran past its deadline: {source}")]
    CancelTimeout {
        #[source]
        source: TransportError,
    },
    #[error("the Vertex batch retrieval after cancel failed: {source}")]
    RetrieveTransport {
        #[source]
        source: TransportError,
    },
    #[error("the Vertex batch retrieval after cancel returned HTTP {status}: {message}")]
    RetrieveAfterCancelStatus { status: u16, message: String },
}

impl VertexBatchLifecycleError {
    // LiteLLM's proxy answers with each provider status it holds, including a failed cancel POST, while an error with no status of its own is a 500
    #[must_use]
    pub fn status_code(&self) -> u16 {
        match self {
            Self::Batch(source) => source.status_code(),
            Self::Records { source } => source.status_code().unwrap_or(500),
            Self::RetrieveStatus { status, .. }
            | Self::ListStatus { status, .. }
            | Self::CancelStatus { status, .. }
            | Self::RetrieveAfterCancelStatus { status, .. } => *status,
            Self::CancelTimeout { .. } => 408,
            Self::Credentials { .. }
            | Self::Header { .. }
            | Self::Segment { .. }
            | Self::Target(_)
            | Self::Url { .. }
            | Self::ProtectedGet { .. }
            | Self::ListTransport { .. }
            | Self::CancelTransport { .. }
            | Self::RetrieveTransport { .. } => 500,
        }
    }

    pub(crate) const fn cancel_transport(source: TransportError) -> Self {
        if matches!(
            &source,
            TransportError::Deadline(_) | TransportError::ConnectTimeout(_)
        ) {
            Self::CancelTimeout { source }
        } else {
            Self::CancelTransport { source }
        }
    }
}
