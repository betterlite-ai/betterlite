use std::time::Duration;

use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};

use super::{SpeechTransport, SpeechWireRequest};

// The pooled client's wire adapter: the request's timeout or the client's own read timeout sets the deadline
impl SpeechTransport for ProxiedClient {
    fn default_timeout(&self) -> Duration {
        self.limits.read_timeout
    }

    async fn send(&self, request: SpeechWireRequest) -> Result<UpstreamResponse, TransportError> {
        let deadline = request.timeout.unwrap_or(self.limits.read_timeout);
        self.send_following(OutboundRequest {
            method: request.method,
            url: request.url,
            query: Vec::new(),
            headers: request.headers,
            body: request.body,
            deadline,
        })
        .await?
        .buffered(self.limits.body_bytes)
        .await
    }
}
