use std::time::Duration;

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;

use super::{BufferedReply, ContentRequest, ContentTransport, StreamedReply};

// The pooled client's wire adapter: the per-call timeout or the client's own read timeout sets the deadline
impl ContentTransport for ProxiedClient {
    async fn send(&self, request: ContentRequest) -> Result<BufferedReply, TransportError> {
        let deadline = request.timeout.unwrap_or(self.limits.read_timeout);
        let stream = self.send_following(outbound(request, deadline)).await?;
        let answered = stream.response.url().clone();
        let reply = stream.buffered(self.limits.body_bytes).await?;
        Ok(BufferedReply {
            status: reply.status,
            url: answered,
            headers: reply.headers,
            body: reply.body,
        })
    }

    async fn send_stream(&self, request: ContentRequest) -> Result<StreamedReply, TransportError> {
        let deadline = request.timeout.unwrap_or(self.limits.read_timeout);
        let stream = self.send_following(outbound(request, deadline)).await?;
        let status = stream.response.status().as_u16();
        let answered = stream.response.url().clone();
        let headers = stream.response.headers().clone();
        Ok(StreamedReply {
            status,
            url: answered,
            headers,
            body: stream.body_stream(),
        })
    }
}

fn outbound(request: ContentRequest, deadline: Duration) -> OutboundRequest {
    OutboundRequest {
        method: request.method,
        url: request.url,
        query: request.query,
        headers: request.headers,
        body: RequestBody::Empty,
        deadline,
    }
}
