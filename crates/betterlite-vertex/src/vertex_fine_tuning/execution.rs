use std::future::Future;

use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use thiserror::Error;

use crate::cache_lookup::error::CacheLookupError;
use crate::endpoint_target::error::EndpointTargetError;
use crate::model_urls::error::VertexUrlError;
use crate::token_cache::error::CredentialCacheError;
use crate::vertex_fine_tuning::records::TuningRecordError;

pub mod create;

#[derive(Debug, Error)]
pub enum TuningExecutionError {
    #[error("the vertex credentials could not be resolved: {0}")]
    Credentials(#[from] CredentialCacheError),
    #[error("the tuning job request could not be prepared: {0}")]
    Target(#[from] EndpointTargetError),
    #[error("the tuning job URL cannot be built: {0}")]
    VertexUrl(#[from] VertexUrlError),
    #[error("the tuning job URL cannot be parsed: {0}")]
    Url(#[from] url::ParseError),
    #[error("a tuning job request header is not valid: {0}")]
    Header(#[from] CacheLookupError),
    #[error("the tuning job request failed: {0}")]
    Transport(#[from] TransportError),
    #[error(transparent)]
    Record(#[from] TuningRecordError),
    #[error("creating the tuning job failed with status {status}: {body}")]
    Http { status: u16, body: String },
}

impl TuningExecutionError {
    // LiteLLM keeps the upstream status in the text of a bare Exception, so the proxy answers 500 for every failure
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::Credentials(_)
            | Self::Target(_)
            | Self::VertexUrl(_)
            | Self::Url(_)
            | Self::Header(_)
            | Self::Transport(_)
            | Self::Record(_)
            | Self::Http { .. } => 500,
        }
    }
}

// B173's pooled client in production and a scripted reply in tests
pub trait TuningTransport: Send + Sync {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send;
}

impl TuningTransport for ProxiedClient {
    // LiteLLM's httpx handlers follow redirects, so the tuning POST re-checks each hop too
    async fn send(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        self.send_following(request)
            .await?
            .buffered(self.limits.body_bytes)
            .await
    }
}

mod tests;
