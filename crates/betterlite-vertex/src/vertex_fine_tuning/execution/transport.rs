use std::future::Future;

use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};

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
