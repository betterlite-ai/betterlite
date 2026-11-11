use std::time::Duration;

use http::Method;
use url::Url;

use crate::pkce_login::{Http, HttpBody, HttpResponse, models::PkceFailure};

use super::{Client, ClientError, Exchange, Payload, Request};

impl Http for Client {
    fn get(&self, url: &str, timeout_seconds: u64) -> Result<HttpResponse, PkceFailure> {
        exchange_pkce(self, Method::GET, url, Payload::Empty, timeout_seconds)
    }

    // The transport never follows a redirect for either flag, so a 3xx comes back as the answer the caller checks.
    fn post(
        &self,
        url: &str,
        body: HttpBody,
        timeout_seconds: u64,
        _follow_redirects: bool,
    ) -> Result<HttpResponse, PkceFailure> {
        let payload = match body {
            HttpBody::Json(bytes) => Payload::Json(bytes),
            HttpBody::Form(fields) => Payload::Form(fields),
        };
        exchange_pkce(self, Method::POST, url, payload, timeout_seconds)
    }
}

fn exchange_pkce(
    client: &Client,
    method: Method,
    url: &str,
    payload: Payload,
    timeout_seconds: u64,
) -> Result<HttpResponse, PkceFailure> {
    let request = Request {
        method,
        url: Url::parse(url).map_err(|source| PkceFailure::InvalidBaseUrl { source })?,
        headers: Vec::new(),
        payload,
    };
    client
        .exchange(Exchange {
            transport: &client.without_redirects,
            request,
            timeout: Duration::from_secs(timeout_seconds),
            authorization: None,
        })
        .map_err(pkce_failure)
}

fn pkce_failure(error: ClientError) -> PkceFailure {
    match error {
        ClientError::Url { source, .. } => PkceFailure::InvalidBaseUrl { source },
        ClientError::Client(source) | ClientError::Transport(source) => {
            PkceFailure::Transport { source }
        }
        ClientError::ProxySettings(source) => PkceFailure::Protocol(source.to_string()),
        ClientError::Runtime(source) => PkceFailure::Protocol(source.to_string()),
        ClientError::HeaderName { source, .. } => PkceFailure::Protocol(source.to_string()),
        ClientError::HeaderValue { source, .. } => PkceFailure::Protocol(source.to_string()),
    }
}
