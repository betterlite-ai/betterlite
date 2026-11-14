use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::user_url::error::UserUrlError;
use thiserror::Error;

use crate::endpoint_target::error::EndpointTargetError;
use crate::token_cache::error::CredentialCacheError;
use crate::vertex_batch_records::error::BatchRecordsError;

#[derive(Debug, Error)]
pub enum VertexBatchCreateError {
    #[error(
        "Vertex AI batch prediction is not supported for `custom_endpoint` deployments. The OpenAI-compatible custom endpoint path has no batch surface in LiteLLM; use a publisher model or fine-tuned Gemini endpoint deployment instead."
    )]
    CustomEndpoint,
    #[error("the Vertex credentials could not be resolved: {source}")]
    Credentials {
        #[source]
        source: CredentialCacheError,
    },
    #[error("the Authorization header cannot carry the access token")]
    Header {
        #[source]
        source: http::header::InvalidHeaderValue,
    },
    #[error("the batch job cannot be built: {source}")]
    Records {
        #[source]
        source: BatchRecordsError,
    },
    #[error(transparent)]
    Target(#[from] EndpointTargetError),
    #[error("the batch URL is not valid: {source}")]
    Url {
        #[source]
        source: url::ParseError,
    },
    #[error("the batch request could not be encoded: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the batch request failed: {source}")]
    Transport {
        #[source]
        source: TransportError,
    },
    #[error("the batch request ran past its deadline: {source}")]
    Timeout {
        #[source]
        source: TransportError,
    },
    #[error("the tuned endpoint lookup was refused: {source}")]
    Lookup {
        #[source]
        source: UserUrlError,
    },
    #[error("the tuned endpoint lookup ran past its deadline")]
    LookupTimeout,
    #[error("Failed to resolve fine-tuned Vertex endpoint '{model}': {message}")]
    EndpointStatus {
        status: u16,
        model: String,
        message: String,
    },
    #[error(
        "Vertex endpoint '{model}' has no deployed model, so there is no tuned model resource to run batch predictions against"
    )]
    NoDeployedModel { model: String },
    #[error("the batch create request returned HTTP {status}: {message}")]
    Http { status: u16, message: String },
    #[error("a Vertex batch payload is larger than {limit} bytes")]
    PayloadTooLarge { limit: usize },
    #[error("the Vertex batch payload at {path} is not valid: {source}")]
    Decode {
        path: String,
        #[source]
        source: serde_json::Error,
    },
}

impl VertexBatchCreateError {
    // LiteLLM raises VertexAIError with 400 or the upstream status and a create timeout becomes 408; every other failure reaches the proxy as a 500
    #[must_use]
    pub fn status_code(&self) -> u16 {
        match self {
            Self::CustomEndpoint | Self::NoDeployedModel { .. } => 400,
            Self::Records { source } => source.status_code().unwrap_or(500),
            Self::EndpointStatus { status, .. } | Self::Http { status, .. } => *status,
            Self::Timeout { .. } => 408,
            Self::Credentials { .. }
            | Self::Header { .. }
            | Self::Target(_)
            | Self::Url { .. }
            | Self::Encode { .. }
            | Self::Transport { .. }
            | Self::Lookup { .. }
            | Self::LookupTimeout
            | Self::PayloadTooLarge { .. }
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
