use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::Method;
use http::header::HeaderMap;
use thiserror::Error;
use url::{ParseError, Url};

#[derive(Clone, Debug)]
pub struct AuthPost {
    pub url: String,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
    pub timeout: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum AuthFailure {
    #[error("the AgentOps authentication URL is not valid")]
    Url(#[source] ParseError),
    #[error("the AgentOps authentication request could not be completed")]
    Request(#[source] TransportError),
}

#[async_trait]
pub trait AuthTransport: Send + Sync {
    async fn post(&self, request: AuthPost) -> Result<AuthResponse, AuthFailure>;
}

// the shared client from B173, so the token request reuses its TLS, proxy and connection pool
#[derive(Clone, Debug)]
pub struct PooledAuthTransport {
    pub client: ProxiedClient,
}

#[async_trait]
impl AuthTransport for PooledAuthTransport {
    async fn post(&self, request: AuthPost) -> Result<AuthResponse, AuthFailure> {
        let url = Url::parse(&request.url).map_err(AuthFailure::Url)?;
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
            .map_err(AuthFailure::Request)?;
        Ok(AuthResponse {
            status: response.status,
            body: response.body,
        })
    }
}
