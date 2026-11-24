use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_core::api::file_http_content::service::{
    BufferedReply, ClientSource, ContentRequest, ContentTransport, Reply, StreamedReply,
};
use betterlite_core::api::vector_store_http_write::error::ClientError;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, StreamResponse};
use http::HeaderMap;
use http::header::{HeaderName, HeaderValue};
use url::Url;

// The pooled client B2463's handler would create; a supplied HTTPHandler lends its headers instead
pub(crate) struct ContentSource {
    pub(crate) client: ProxiedClient,
    pub(crate) headers: BTreeMap<String, String>,
}

impl ClientSource for ContentSource {
    type Client = WireClient;

    fn client(&self) -> Result<Self::Client, ClientError> {
        Ok(WireClient {
            client: self.client.clone(),
            headers: self.headers.clone(),
        })
    }
}

// A supplied HTTPHandler's own httpx client sends every request with its headers, so the wire carries them
pub(crate) struct WireClient {
    pub(crate) client: ProxiedClient,
    pub(crate) headers: BTreeMap<String, String>,
}

impl ContentTransport for WireClient {
    async fn send(&self, request: ContentRequest) -> Result<BufferedReply, TransportError> {
        let stream = self.open(request).await?;
        let url = stream.response.url().clone();
        let reply = stream.buffered(self.client.limits.body_bytes).await?;
        Ok(reply_with(url, reply.status, reply.headers, reply.body))
    }

    async fn send_stream(&self, request: ContentRequest) -> Result<StreamedReply, TransportError> {
        let stream = self.open(request).await?;
        let url = stream.response.url().clone();
        let status = stream.response.status().as_u16();
        let headers = stream.response.headers().clone();
        Ok(reply_with(url, status, headers, stream.body_stream()))
    }
}

impl WireClient {
    async fn open(&self, request: ContentRequest) -> Result<StreamResponse, TransportError> {
        let deadline = request.timeout.unwrap_or(self.client.limits.read_timeout);
        self.client
            .send_following(outbound(self, request, deadline))
            .await
    }
}

// the answered URL is the response's own, since a redirect changes it from the requested one
const fn reply_with<B>(url: Url, status: u16, headers: HeaderMap, body: B) -> Reply<B> {
    Reply {
        status,
        url,
        headers,
        body,
    }
}

fn outbound(
    client: &WireClient,
    mut request: ContentRequest,
    deadline: Duration,
) -> OutboundRequest {
    let lent = client.headers.iter().filter_map(|(name, value)| {
        let header_name = HeaderName::from_bytes(name.as_bytes()).ok()?;
        let header_value = HeaderValue::from_str(value).ok()?;
        Some((header_name, header_value))
    });
    request.headers.extend(lent);
    OutboundRequest {
        method: request.method,
        url: request.url,
        query: request.query,
        headers: request.headers,
        body: RequestBody::Empty,
        deadline,
    }
}
