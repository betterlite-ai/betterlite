use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use http::Method;

use super::service::{EditRequest, EditTransport};

// The pooled client's wire adapter: the request's timeout sets the deadline and the client's body limit caps the answer
impl EditTransport for ProxiedClient {
    async fn send(&self, request: EditRequest) -> Result<UpstreamResponse, TransportError> {
        self.send_following(OutboundRequest {
            method: Method::POST,
            url: request.url,
            query: Vec::new(),
            headers: request.headers,
            body: request.body,
            deadline: request.timeout,
        })
        .await?
        .buffered(self.limits.body_bytes)
        .await
    }
}
