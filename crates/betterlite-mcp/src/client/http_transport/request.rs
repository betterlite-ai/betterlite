use std::collections::BTreeMap;
use std::time::Duration;

use bytes::Bytes;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use http::{Method, Uri};
use sha2::{Digest as _, Sha256};
use url::Url;

use betterlite_http::mcp_sigv4::McpSigV4Auth;

use crate::client::sessions::factory::{HttpClientConfig, RequestCredential};
use crate::credentials;

use super::error::McpHttpError;

const CONTENT_SHA256: &str = "x-amz-content-sha256";

pub struct McpHttpRequest {
    pub method: Method,
    pub url: Url,
    pub headers: HeaderMap,
    pub body: Bytes,
    // The legacy SSE GET reads with its own timeout while connect and writes keep the client's
    pub read_timeout: Option<Duration>,
}

pub(super) struct PreparedRequest {
    pub(super) method: Method,
    pub(super) url: Url,
    pub(super) headers: HeaderMap,
    pub(super) body: Bytes,
}

// LiteLLM applies the auth flow first and the request hooks to the request it yields, so the guard runs after auth
pub(super) async fn prepare(
    config: &HttpClientConfig,
    request: &McpHttpRequest,
) -> Result<PreparedRequest, McpHttpError> {
    let mut headers = configured_headers(&config.headers)?;
    for (name, value) in &request.headers {
        headers.insert(name.clone(), value.clone());
    }
    let mut method = request.method.clone();
    let mut url = request.url.clone();
    match config.credential.as_ref() {
        Some(RequestCredential::Resolved(auth)) => {
            let outcome = auth.authenticate(credentials::Request {
                method: method.as_str().to_owned(),
                url: url.as_str().to_owned(),
                headers: header_text(&headers)?,
            });
            // httpx closes the auth flow after the request it yielded, as prepare_request_auth already does
            auth.close();
            let authenticated = outcome?;
            method = Method::from_bytes(authenticated.method.as_bytes())
                .map_err(McpHttpError::CredentialMethod)?;
            url = Url::parse(&authenticated.url).map_err(McpHttpError::CredentialUrl)?;
            headers = configured_headers(&authenticated.headers)?;
        }
        Some(RequestCredential::AwsSigV4(signer)) => {
            headers = signed_headers(signer, &method, &url, headers, &request.body).await?;
        }
        None => {}
    }
    if let Some(guard) = config.redirect_guard.as_ref() {
        let mut text = header_text(&headers)?;
        guard.apply(url.as_str(), &mut text);
        headers = configured_headers(&text)?;
    }
    Ok(PreparedRequest {
        method,
        url,
        headers,
        body: request.body.clone(),
    })
}

async fn signed_headers(
    signer: &McpSigV4Auth,
    method: &Method,
    url: &Url,
    headers: HeaderMap,
    body: &Bytes,
) -> Result<HeaderMap, McpHttpError> {
    let mut request = http::Request::new(body.clone());
    *request.method_mut() = method.clone();
    *request.uri_mut() = url.as_str().parse::<Uri>().map_err(McpHttpError::Uri)?;
    *request.headers_mut() = headers;
    // botocore's SigV4Auth puts the body hash on the wire; aws-sigv4 only reads it from the headers
    let digest = hex::encode(Sha256::digest(body));
    let content_hash =
        HeaderValue::from_bytes(digest.as_bytes()).map_err(|source| McpHttpError::HeaderValue {
            name: CONTENT_SHA256.to_owned(),
            source,
        })?;
    request
        .headers_mut()
        .entry(HeaderName::from_static(CONTENT_SHA256))
        .or_insert(content_hash);
    let signed_request = signer.auth_flow(request).await?;
    Ok(signed_request.headers().clone())
}

fn configured_headers(headers: &BTreeMap<String, String>) -> Result<HeaderMap, McpHttpError> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let header_name =
            HeaderName::from_bytes(name.as_bytes()).map_err(|source| McpHttpError::HeaderName {
                name: name.clone(),
                source,
            })?;
        let header_value =
            HeaderValue::from_str(value).map_err(|source| McpHttpError::HeaderValue {
                name: name.clone(),
                source,
            })?;
        map.insert(header_name, header_value);
    }
    Ok(map)
}

fn header_text(headers: &HeaderMap) -> Result<BTreeMap<String, String>, McpHttpError> {
    headers
        .iter()
        .map(|(name, value)| {
            let visible = value.to_str().map_err(|source| McpHttpError::HeaderText {
                name: name.as_str().to_owned(),
                source,
            })?;
            Ok((name.as_str().to_owned(), visible.to_owned()))
        })
        .collect()
}
