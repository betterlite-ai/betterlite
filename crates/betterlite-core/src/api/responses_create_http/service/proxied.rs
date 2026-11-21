use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;

use super::source::WireSource;
use super::{CreateRequest, CreateTransport, SentReply};

// The pooled client's wire adapter: a streaming request keeps its body open, anything else is buffered
impl CreateTransport for ProxiedClient {
    async fn send(
        &self,
        request: CreateRequest,
        stream: bool,
    ) -> Result<SentReply, TransportError> {
        let response = self
            .send_following(OutboundRequest {
                method: request.method,
                url: request.url,
                query: Vec::new(),
                headers: request.headers,
                body: request.body,
                deadline: request.timeout,
            })
            .await?;
        if stream && (200..=299).contains(&response.response.status().as_u16()) {
            return Ok(SentReply::Streaming(Box::new(WireSource::new(response))));
        }
        Ok(SentReply::Buffered(
            response.buffered(self.limits.body_bytes).await?,
        ))
    }
}
