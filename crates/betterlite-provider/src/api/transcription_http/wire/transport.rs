use std::time::Duration;

use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use http::Method;
use http::header::HeaderMap;
use url::Url;

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
            TranscriptionBody::Encoded(encoded) => {
                self.send_following(OutboundRequest {
                    method: Method::POST,
                    url,
                    query: Vec::new(),
                    headers,
                    body: encoded,
                    deadline,
                })
                .await?
                .buffered(self.limits.body_bytes)
                .await
            }
            TranscriptionBody::Raw(data) => send_raw(self, url, headers, data, deadline).await,
        }
    }
}

// Raw bytes keep the client's own send: betterlite-http has no body variant that adds no Content-Type
async fn send_raw(
    client: &ProxiedClient,
    url: Url,
    headers: HeaderMap,
    data: Vec<u8>,
    deadline: Duration,
) -> Result<UpstreamResponse, TransportError> {
    let mut built = client
        .client
        .post(url)
        .timeout(deadline)
        .headers(headers)
        .body(data)
        .build()
        .map_err(TransportError::Build)?;
    for (_name, value) in built
        .headers_mut()
        .iter_mut()
        .filter(|(name, _value)| SECRET_HEADERS.contains(&name.as_str()))
    {
        value.set_sensitive(true);
    }
    client
        .execute_buffered(built, client.limits.body_bytes)
        .await
        .map_err(TransportError::Rejected)
}
