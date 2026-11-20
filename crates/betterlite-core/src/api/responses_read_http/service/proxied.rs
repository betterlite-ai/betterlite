use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};

use super::{ReadRequest, ReadTransport};

// The pooled client's wire adapter: the handlers pass no timeout, so the client's own read timeout sets the deadline
impl ReadTransport for ProxiedClient {
    async fn send(&self, request: ReadRequest) -> Result<UpstreamResponse, TransportError> {
        self.send_following(OutboundRequest {
            method: request.method,
            url: request.url,
            query: request.query,
            headers: request.headers,
            body: request.body,
            deadline: self.limits.read_timeout,
        })
        .await?
        .buffered(self.limits.body_bytes)
        .await
    }
}
