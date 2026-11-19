use std::mem;
use std::time::Duration;

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::Method;

use super::{GenAiRequest, GenAiTransport, StreamReply, WireReply};

// The pooled client's wire adapter: a streaming request keeps its response open, everything else is read whole
impl GenAiTransport for ProxiedClient {
    fn default_timeout(&self) -> Duration {
        self.limits.read_timeout
    }

    async fn send(
        &self,
        request: GenAiRequest,
        streaming: bool,
    ) -> Result<WireReply, TransportError> {
        let deadline = request.timeout.unwrap_or(self.limits.read_timeout);
        let mut reply = self
            .send_following(OutboundRequest {
                method: Method::POST,
                url: request.url,
                query: Vec::new(),
                headers: request.headers,
                body: RequestBody::Json(request.body),
                deadline,
            })
            .await?;
        if streaming && reply.response.status().as_u16() < 400 {
            let headers = mem::take(reply.response.headers_mut());
            return Ok(WireReply::Streaming(StreamReply {
                headers,
                body: reply.body_stream(),
            }));
        }
        reply
            .buffered(self.limits.body_bytes)
            .await
            .map(WireReply::Buffered)
    }
}
