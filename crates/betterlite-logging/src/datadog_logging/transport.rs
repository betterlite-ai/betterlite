use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::Method;
use http::header::{HeaderMap, HeaderValue};
use thiserror::Error;
use url::{ParseError, Url};

// LiteLLM's cached logging client uses its fallback total, so a dead intake cannot wedge a flush forever
pub const LOGGING_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Debug)]
pub struct PostRequest {
    pub url: String,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
    pub timeout: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransportResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum TransportFailure {
    #[error("the Datadog request URL is not valid")]
    Url(#[source] ParseError),
    #[error("the Datadog request could not be completed")]
    Request(#[source] TransportError),
}

#[async_trait]
pub trait Transport: Send + Sync {
    async fn post(&self, request: PostRequest) -> Result<TransportResponse, TransportFailure>;
}

// the logging client a Datadog logger gets from B173's pool, so posts share its TLS, proxy and limits
#[derive(Clone, Debug)]
pub struct PooledTransport {
    pub client: ProxiedClient,
}

#[async_trait]
impl Transport for PooledTransport {
    async fn post(&self, request: PostRequest) -> Result<TransportResponse, TransportFailure> {
        let url = Url::parse(&request.url).map_err(TransportFailure::Url)?;
        let outbound = OutboundRequest {
            method: Method::POST,
            url,
            query: Vec::new(),
            headers: request.headers,
            body: RequestBody::Bytes {
                content_type: HeaderValue::from_static("application/json"),
                data: request.body,
            },
            deadline: request.timeout,
        };
        let response = self
            .client
            .send_buffered(outbound)
            .await
            .map_err(TransportFailure::Request)?;
        Ok(TransportResponse {
            status: response.status,
            body: response.body,
        })
    }
}
