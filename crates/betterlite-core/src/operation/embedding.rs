use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::embedding::{
    EmbeddingError, EmbeddingRequest as ClientEmbeddingRequest, EmbeddingResponse,
};
use http::HeaderMap;
use thiserror::Error;

pub struct EmbeddingRequest {
    pub call_id: String,
    pub model_id: String,
    pub request: Box<ClientEmbeddingRequest>,
    pub calls: Arc<dyn EmbeddingCall>,
}

// the existing embedding implementation sits behind this seam, so the pipeline stays in betterlite-core
#[async_trait]
pub trait EmbeddingCall: Send + Sync {
    async fn embed(
        &self,
        call: EmbeddingCallInput<'_>,
    ) -> Result<EmbeddingResult, EmbeddingFailure>;
}

pub struct EmbeddingCallInput<'call> {
    pub deployment: ChatDeployment,
    pub request: &'call ClientEmbeddingRequest,
}

pub struct EmbeddingResult {
    pub response: EmbeddingResponse,
    pub upstream_headers: HeaderMap,
    pub upstream_time: Duration,
}

#[derive(Debug, Error)]
pub enum EmbeddingFailure {
    #[error(transparent)]
    Route(#[from] EmbeddingError),
    #[error("{message}")]
    Internal { message: String },
    #[error("the provider answered a different operation than the one sent")]
    Mismatched,
}

impl EmbeddingFailure {
    pub(crate) fn status(&self) -> u16 {
        match self {
            Self::Route(route) => route_status(route),
            Self::Internal { .. } | Self::Mismatched => 500,
        }
    }

    pub(crate) const fn provider_originated(&self) -> bool {
        match self {
            Self::Route(EmbeddingError::Provider { .. }) => true,
            Self::Route(_) | Self::Internal { .. } | Self::Mismatched => false,
        }
    }
}

fn route_status(error: &EmbeddingError) -> u16 {
    match error {
        EmbeddingError::Body { .. }
        | EmbeddingError::Unsupported { .. }
        | EmbeddingError::UnknownModel { .. } => 400,
        EmbeddingError::MissingApiKey => 401,
        EmbeddingError::UnsupportedProvider { .. } => 501,
        EmbeddingError::Provider { response } => response.status,
        EmbeddingError::Encode { .. }
        | EmbeddingError::Decode { .. }
        | EmbeddingError::Endpoint { .. }
        | EmbeddingError::Http { .. }
        | EmbeddingError::Mock { .. } => 500,
    }
}
