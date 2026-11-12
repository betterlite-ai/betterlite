use std::error::Error;
use std::future::Future;
use std::time::Duration;

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::Method;
use http::header::HeaderMap;
use url::Url;

// LiteLLM's shared logging client waits its default timeout, which no cost upload overrides
pub const UPLOAD_DEADLINE: Duration = Duration::from_secs(600);

pub struct PutRequest {
    pub url: Url,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

pub struct PutResponse {
    pub status: u16,
}

pub trait Transport: Send + Sync {
    type Error: Error + Send + Sync + 'static;

    fn put(
        &self,
        request: PutRequest,
    ) -> impl Future<Output = Result<PutResponse, Self::Error>> + Send;
}

pub struct PooledTransport {
    pub client: ProxiedClient,
}

impl Transport for PooledTransport {
    type Error = TransportError;

    fn put(
        &self,
        request: PutRequest,
    ) -> impl Future<Output = Result<PutResponse, Self::Error>> + Send {
        let outbound = OutboundRequest {
            method: Method::PUT,
            url: request.url,
            query: Vec::new(),
            headers: request.headers,
            body: RequestBody::Json(request.body),
            deadline: UPLOAD_DEADLINE,
        };
        async move {
            // LiteLLM's logging client follows redirects, so a moved endpoint still receives the cost entries
            let response = self.client.send_following(outbound).await?;
            let buffered = response.buffered(self.client.limits.body_bytes).await?;
            Ok(PutResponse {
                status: buffered.status,
            })
        }
    }
}
