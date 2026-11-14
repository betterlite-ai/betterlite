use std::collections::BTreeMap;
use std::future::Future;
use std::time::Duration;

use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use http::Method;
use http::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use crate::endpoint_target::target::{AuthHeader, EndpointTarget};

use super::error::GeminiEmbeddingCallError;

// Native file lookups always go to Google's own host, even when embeddings use a custom api_base
const FILE_BASE: &str = "https://generativelanguage.googleapis.com/v1beta/";
const API_KEY_HEADER: &str = "x-goog-api-key";

// B173's pooled client in production and a scripted reply in tests
pub trait EmbeddingTransport: Send + Sync {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send;
}

impl EmbeddingTransport for ProxiedClient {
    async fn send(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        // LiteLLM's httpx handlers follow redirects, so each hop is checked again
        self.send_following(request)
            .await?
            .buffered(self.limits.body_bytes)
            .await
    }
}

fn header_failure<Source>(name: &str, source: Source) -> GeminiEmbeddingCallError
where
    http::Error: From<Source>,
{
    GeminiEmbeddingCallError::Header {
        name: name.to_owned(),
        source: http::Error::from(source),
    }
}

fn secret_value(name: &str, secret: &str) -> Result<HeaderValue, GeminiEmbeddingCallError> {
    let mut value = HeaderValue::from_str(secret).map_err(|source| header_failure(name, source))?;
    value.set_sensitive(true);
    Ok(value)
}

fn parse_url(text: &str) -> Result<Url, GeminiEmbeddingCallError> {
    Url::parse(text).map_err(|source| GeminiEmbeddingCallError::Url {
        url: text.to_owned(),
        source,
    })
}

// A GET carries only the key; the file reference is appended to the fixed base as written.
pub fn file_request(
    file: &str,
    api_key: &SecretString,
    deadline: Duration,
) -> Result<OutboundRequest, GeminiEmbeddingCallError> {
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static(API_KEY_HEADER),
        secret_value(API_KEY_HEADER, api_key.expose_secret())?,
    );
    Ok(OutboundRequest {
        method: Method::GET,
        url: parse_url(&format!("{FILE_BASE}{file}"))?,
        query: Vec::new(),
        headers,
        body: RequestBody::Empty,
        deadline,
    })
}

// Content-Type first, then the auth fields, then the caller's headers, so the caller wins every clash.
pub fn embedding_request(
    target: &EndpointTarget,
    extra_headers: &BTreeMap<String, String>,
    body: Vec<u8>,
    deadline: Duration,
) -> Result<OutboundRequest, GeminiEmbeddingCallError> {
    let mut headers = HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    match &target.auth {
        None => {}
        Some(AuthHeader::Token(token)) => {
            let bearer = format!("Bearer {}", token.expose_secret());
            headers.insert(AUTHORIZATION, secret_value("Authorization", &bearer)?);
        }
        Some(AuthHeader::Headers(entries)) => {
            for (name, secret) in entries {
                let header = HeaderName::from_bytes(name.as_bytes())
                    .map_err(|source| header_failure(name, source))?;
                headers.insert(header, secret_value(name, secret.expose_secret())?);
            }
        }
    }
    for (name, value) in extra_headers {
        let header = HeaderName::from_bytes(name.as_bytes())
            .map_err(|source| header_failure(name, source))?;
        let text = HeaderValue::from_str(value).map_err(|source| header_failure(name, source))?;
        headers.insert(header, text);
    }
    Ok(OutboundRequest {
        method: Method::POST,
        url: parse_url(&target.url)?,
        query: Vec::new(),
        headers,
        body: RequestBody::Json(body),
        deadline,
    })
}
