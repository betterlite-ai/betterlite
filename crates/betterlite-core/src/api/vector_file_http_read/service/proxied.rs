use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;

use super::{ReadRequest, ReadTransport};

// The pooled client's wire adapter: the request's timeout or the client's own read timeout sets the deadline
impl ReadTransport for ProxiedClient {
    async fn send(&self, request: ReadRequest) -> Result<UpstreamResponse, TransportError> {
        let deadline = request.timeout.unwrap_or(self.limits.read_timeout);
        self.send_following(OutboundRequest {
            method: request.method,
            url: request.url,
            query: request.query,
            headers: request.headers,
            body: RequestBody::Empty,
            deadline,
        })
        .await?
        .buffered(self.limits.body_bytes)
        .await
    }
}
