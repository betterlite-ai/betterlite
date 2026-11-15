use std::future::Future;

use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};

pub trait ImageVariationTransport: Send + Sync {
    fn create_variation(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send;
}

impl ImageVariationTransport for ProxiedClient {
    async fn create_variation(
        &self,
        request: OutboundRequest,
    ) -> Result<UpstreamResponse, TransportError> {
        let stream = self.send_following(request).await?;
        stream.buffered(self.limits.body_bytes).await
    }
}
