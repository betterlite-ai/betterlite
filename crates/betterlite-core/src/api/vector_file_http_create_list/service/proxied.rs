use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};

use super::{FileRequest, FileTransport};

// The pooled client's wire adapter: the create timeout or the client's own read timeout sets the deadline, and the list query rides the request
impl FileTransport for ProxiedClient {
    async fn send(&self, request: FileRequest) -> Result<UpstreamResponse, TransportError> {
        let deadline = request.timeout.unwrap_or(self.limits.read_timeout);
        let stream = self
            .send_following(OutboundRequest {
                method: request.method,
                url: request.url,
                query: request.query,
                headers: request.headers,
                body: request.body,
                deadline,
            })
            .await?;
        stream.buffered(self.limits.body_bytes).await
    }
}
