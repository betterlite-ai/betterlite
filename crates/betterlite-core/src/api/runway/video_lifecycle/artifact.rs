use std::future::Future;

use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};

// LiteLLM downloads the artifact through its own httpx client, so the transport is a dependency, not a method
pub trait ArtifactTransport: Sync {
    fn fetch(
        &self,
        request: OutboundRequest,
        max_bytes: usize,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send;
}

impl ArtifactTransport for ProxiedClient {
    async fn fetch(
        &self,
        request: OutboundRequest,
        max_bytes: usize,
    ) -> Result<UpstreamResponse, TransportError> {
        self.send_following(request)
            .await?
            .buffered(max_bytes)
            .await
    }
}
