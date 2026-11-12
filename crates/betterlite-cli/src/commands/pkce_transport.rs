use std::time::Duration;

use http::Method;
use http::header::HeaderMap;
use url::Url;

use crate::pkce_login::{Http, HttpBody, HttpResponse, models::PkceFailure};

use super::{Client, Payload, PreparedRequest, encode_payload};

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
    target: &str,
    payload: Payload,
    timeout_seconds: u64,
) -> Result<HttpResponse, PkceFailure> {
    let url = Url::parse(target).map_err(|source| PkceFailure::InvalidBaseUrl { source })?;
    let mut header_map = HeaderMap::new();
    let body = encode_payload(payload, &mut header_map);
    let prepared = PreparedRequest {
        method,
        url,
        headers: header_map,
        body,
        timeout: Duration::from_secs(timeout_seconds),
    };
    client
        .send_request(&client.without_redirects, prepared)
        .map_err(|source| PkceFailure::Transport { source })
}
