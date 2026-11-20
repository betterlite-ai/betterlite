use betterlite_http::error::HttpError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use futures_util::StreamExt as _;
use tokio::time::timeout;

use super::{UploadBody, UploadRequest, UploadTransport};

// The pooled client's wire adapter: multipart follows redirects, every other body goes out as built
impl UploadTransport for ProxiedClient {
    async fn send(&self, request: UploadRequest) -> Result<UpstreamResponse, TransportError> {
        let deadline = request.timeout.unwrap_or(self.limits.read_timeout);
        let data = match request.body {
            UploadBody::Multipart { boundary, parts } => {
                return self
                    .send_following(OutboundRequest {
                        method: request.method,
                        url: request.url,
                        query: Vec::new(),
                        headers: request.headers,
                        body: RequestBody::Multipart { boundary, parts },
                        deadline,
                    })
                    .await?
                    .buffered(self.limits.body_bytes)
                    .await;
            }
            UploadBody::Bytes(data) => data,
            // B173's outbound body has no streaming form, so the media blocks are gathered first
            UploadBody::Blocks(blocks) => {
                let chunks = timeout(deadline, blocks.collect::<Vec<Vec<u8>>>())
                    .await
                    .map_err(|_elapsed| TransportError::Rejected(HttpError::Timeout))?;
                chunks.concat()
            }
        };
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
}
