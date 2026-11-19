use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use http::Method;

use super::{TranscriptionBody, TranscriptionRequest, TranscriptionTransport};

// The same headers betterlite-http marks sensitive when it builds its own outbound request
const SECRET_HEADERS: [&str; 4] = [
    "authorization",
    "proxy-authorization",
    "x-api-key",
    "api-key",
];

impl TranscriptionTransport for ProxiedClient {
    async fn send(
        &self,
        request: TranscriptionRequest,
    ) -> Result<UpstreamResponse, TransportError> {
        let deadline = request.timeout.unwrap_or(self.limits.read_timeout);
        let TranscriptionRequest {
            url, headers, body, ..
        } = request;
        match body {
            TranscriptionBody::Encoded(body) => {
                self.send_following(OutboundRequest {
                    method: Method::POST,
                    url,
                    query: Vec::new(),
                    headers,
                    body,
                    deadline,
                })
                .await?
                .buffered(self.limits.body_bytes)
                .await
            }
            TranscriptionBody::Raw(data) => {
                let mut built = self
                    .client
                    .post(url)
                    .timeout(deadline)
                    .headers(headers)
                    .body(data)
                    .build()
                    .map_err(TransportError::Build)?;
                for (name, value) in built.headers_mut().iter_mut() {
                    if SECRET_HEADERS.contains(&name.as_str()) {
                        value.set_sensitive(true);
                    }
                }
                self.execute_buffered(built, self.limits.body_bytes)
                    .await
                    .map_err(TransportError::Rejected)
            }
        }
    }
}
