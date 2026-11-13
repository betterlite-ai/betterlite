use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::HeaderMap;
use http::Method;
use http::header::HeaderValue;
use url::Url;

// LiteLLM's shared logging client waits COMPLETION_HTTP_FALLBACK_SECONDS unless litellm.request_timeout is set
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(600);

pub struct UploadRequest {
    pub method: Method,
    pub url: Url,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
    pub timeout: Duration,
    // LiteLLM's shared client follows redirects by default, and only the presigned PUT opts out
    pub follow_redirects: bool,
}

pub struct UploadResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

#[async_trait]
pub trait UploadHttp: Send + Sync {
    async fn send(&self, request: UploadRequest) -> Result<UploadResponse, TransportError>;
}

#[derive(Clone)]
pub struct PointFiveHttp {
    pub client: ProxiedClient,
}

#[async_trait]
impl UploadHttp for PointFiveHttp {
    async fn send(&self, request: UploadRequest) -> Result<UploadResponse, TransportError> {
        let follow_redirects = request.follow_redirects;
        let body = if request.body.is_empty() {
            RequestBody::Empty
        } else {
            RequestBody::Bytes {
                content_type: HeaderValue::from_static("application/octet-stream"),
                data: request.body,
            }
        };
        let outbound = OutboundRequest {
            method: request.method,
            url: request.url,
            query: Vec::new(),
            headers: request.headers,
            body,
            deadline: request.timeout,
        };
        let stream = if follow_redirects {
            self.client.send_following(outbound).await?
        } else {
            self.client.send(outbound).await?
        };
        let response = stream.buffered(self.client.limits.body_bytes).await?;
        Ok(UploadResponse {
            status: response.status,
            body: response.body,
        })
    }
}
