use std::time::Duration;

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::{HeaderMap, HeaderValue, Method};
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use super::codec::REPLY_LIMIT;
use super::error::Failure;

// LiteLLM's default HTTP client waits 600 seconds for an answer
const REQUEST_DEADLINE: Duration = Duration::from_mins(10);

#[derive(Debug, thiserror::Error)]
pub enum HttpFailure {
    #[error("the endpoint address is invalid")]
    Address(#[source] url::ParseError),
    #[error("the transport failed")]
    Transport(#[source] TransportError),
}

pub enum Body {
    Json(SecretString),
    Form(SecretString),
}

pub struct AuthPost<'url> {
    pub url: &'url str,
    pub body: Body,
}

pub struct AuthResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

impl AuthResponse {
    pub fn into_body(self) -> Result<Vec<u8>, Failure> {
        if (200..300).contains(&self.status) {
            Ok(self.body)
        } else {
            Err(Failure::Status(self.status))
        }
    }
}

pub trait AuthTransport {
    fn post(
        &mut self,
        request: AuthPost<'_>,
    ) -> impl Future<Output = Result<AuthResponse, HttpFailure>> + Send;
}

pub struct ProxiedTransport {
    client: ProxiedClient,
}

impl ProxiedTransport {
    #[must_use]
    pub const fn new(mut client: ProxiedClient) -> Self {
        client.limits.body_bytes = REPLY_LIMIT;
        Self { client }
    }
}

impl AuthTransport for ProxiedTransport {
    async fn post(&mut self, request: AuthPost<'_>) -> Result<AuthResponse, HttpFailure> {
        let body = match request.body {
            Body::Json(text) => RequestBody::Json(text.expose_secret().as_bytes().to_vec()),
            Body::Form(text) => RequestBody::Bytes {
                content_type: HeaderValue::from_static("application/x-www-form-urlencoded"),
                data: text.expose_secret().as_bytes().to_vec(),
            },
        };
        let outbound = OutboundRequest {
            method: Method::POST,
            url: Url::parse(request.url).map_err(HttpFailure::Address)?,
            query: Vec::new(),
            headers: HeaderMap::new(),
            body,
            deadline: REQUEST_DEADLINE,
        };
        let upstream = self
            .client
            .send_buffered(outbound)
            .await
            .map_err(HttpFailure::Transport)?;
        Ok(AuthResponse {
            status: upstream.status,
            body: upstream.body,
        })
    }
}

mod tests;
