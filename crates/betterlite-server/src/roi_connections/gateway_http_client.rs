use std::time::Duration;

use axum::http::Method;
use axum::http::header::{AUTHORIZATION, InvalidHeaderValue};
use axum::http::{HeaderMap, HeaderValue};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use secrecy::ExposeSecret as _;
use secrecy::SecretString;
use thiserror::Error;
use url::Url;

use crate::roi_github::transport::Transport;

pub const GATEWAY_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Debug, Error)]
pub enum GatewayError {
    #[error("the gateway credential cannot be carried in a header")]
    Header(#[source] InvalidHeaderValue),
    #[error("the gateway address cannot reach that path")]
    Url(#[source] url::ParseError),
    #[error("the gateway request failed")]
    Transport(#[source] TransportError),
    #[error("the gateway answered with HTTP {status}")]
    Status { status: u16 },
}

#[derive(Clone)]
pub struct GatewayHttpClient<T> {
    pub base: Url,
    pub transport: T,
}

impl<T: Transport> GatewayHttpClient<T> {
    pub async fn request(
        &self,
        method: Method,
        path: &str,
        credential: &SecretString,
        body: RequestBody,
    ) -> Result<UpstreamResponse, GatewayError> {
        let url = self.base.join(path).map_err(GatewayError::Url)?;
        let bearer = format!("Bearer {}", credential.expose_secret());
        let mut authorization = HeaderValue::from_str(&bearer).map_err(GatewayError::Header)?;
        authorization.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, authorization);
        let request = OutboundRequest {
            method,
            url,
            query: Vec::new(),
            headers,
            body,
            deadline: GATEWAY_TIMEOUT,
        };
        let response = self
            .transport
            .send(request)
            .await
            .map_err(GatewayError::Transport)?;
        if response.status >= 400 {
            return Err(GatewayError::Status {
                status: response.status,
            });
        }
        Ok(response)
    }
}

mod tests;
