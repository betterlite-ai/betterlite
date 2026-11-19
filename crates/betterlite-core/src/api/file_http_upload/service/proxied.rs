use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use futures_util::StreamExt as _;
use reqwest::Body;

use super::{UploadBody, UploadRequest, UploadTransport};

// The pooled client's wire adapter. A multipart body rides the redirect-aware path; a text, bytes or
// streamed body goes out exactly as the transform built it, so no default content type is added
impl UploadTransport for ProxiedClient {
    async fn send(&self, request: UploadRequest) -> Result<UpstreamResponse, TransportError> {
        let deadline = request.timeout.unwrap_or(self.limits.read_timeout);
        match request.body {
            UploadBody::Multipart { boundary, parts } => {
                self.send_following(OutboundRequest {
                    method: request.method,
                    url: request.url,
                    query: Vec::new(),
                    headers: request.headers,
                    body: RequestBody::Multipart { boundary, parts },
                    deadline,
                })
                .await?
                .buffered(self.limits.body_bytes)
                .await
            }
            UploadBody::Bytes(data) => {
                let built = self
                    .client
                    .request(request.method, request.url)
                    .timeout(deadline)
                    .headers(request.headers)
                    .body(data)
                    .build()
                    .map_err(TransportError::Build)?;
                self.execute_buffered(built, self.limits.body_bytes)
                    .await
                    .map_err(TransportError::Rejected)
            }
            UploadBody::Blocks(blocks) => {
                let chunks = blocks.map(|block| Ok::<Vec<u8>, std::io::Error>(block));
                let built = self
                    .client
                    .request(request.method, request.url)
                    .timeout(deadline)
                    .headers(request.headers)
                    .body(Body::wrap_stream(chunks))
                    .build()
                    .map_err(TransportError::Build)?;
                self.execute_buffered(built, self.limits.body_bytes)
                    .await
                    .map_err(TransportError::Rejected)
            }
        }
    }
}
