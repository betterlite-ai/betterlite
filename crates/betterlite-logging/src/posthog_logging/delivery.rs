use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::Method;
use http::header::HeaderMap;
use thiserror::Error;
use url::{ParseError, Url};

// LiteLLM's cached logging client falls back to a total timeout, so a dead PostHog cannot wedge a flush forever
pub const LOGGING_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Debug)]
pub struct PostRequest {
    pub url: String,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
    pub timeout: Duration,
}

#[derive(Debug, Error)]
pub enum PostFailure {
    #[error("the PostHog request URL is not valid")]
    Url(#[source] ParseError),
    #[error("the PostHog request could not be completed")]
    Request(#[source] TransportError),
}

#[async_trait]
pub trait Transport: Send + Sync {
    async fn post(&self, request: PostRequest) -> Result<u16, PostFailure>;
}

// the logging client a PostHog logger gets from B173's pool, so posts share its TLS, proxy and limits
#[derive(Clone, Debug)]
pub struct PooledTransport {
    pub client: ProxiedClient,
}

#[async_trait]
impl Transport for PooledTransport {
    async fn post(&self, request: PostRequest) -> Result<u16, PostFailure> {
        let url = Url::parse(&request.url).map_err(PostFailure::Url)?;
        let outbound = OutboundRequest {
            method: Method::POST,
            url,
            query: Vec::new(),
            headers: request.headers,
            body: RequestBody::Json(request.body),
            deadline: request.timeout,
        };
        let response = self
            .client
            .send_buffered(outbound)
            .await
            .map_err(PostFailure::Request)?;
        Ok(response.status)
    }
}
