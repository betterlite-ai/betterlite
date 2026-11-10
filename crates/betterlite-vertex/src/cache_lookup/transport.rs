use std::collections::BTreeMap;
use std::future::Future;
use std::time::Duration;

use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use http::Method;
use http::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use secrecy::ExposeSecret as _;
use url::Url;

use crate::endpoint_target::target::{AuthHeader, EndpointTarget};

use super::error::CacheLookupError;

// B173's pooled client in production and a scripted reply in tests
pub trait CacheTransport: Send + Sync {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send;
}

impl CacheTransport for ProxiedClient {
    async fn send(&self, request: OutboundRequest) -> Result<UpstreamResponse, TransportError> {
        // LiteLLM's httpx handlers follow redirects, so the list path must re-check each hop too
        self.send_following(request)
            .await?
            .buffered(self.limits.body_bytes)
            .await
    }
}

pub fn cache_request(
    target: &EndpointTarget,
    page_token: Option<&str>,
    extra_headers: &BTreeMap<String, String>,
    deadline: Duration,
) -> Result<OutboundRequest, CacheLookupError> {
    let text = page_url(&target.url, page_token);
    let url = Url::parse(&text).map_err(|source| CacheLookupError::Url { source })?;
    Ok(OutboundRequest {
        method: Method::GET,
        url,
        query: Vec::new(),
        headers: cache_headers(target, extra_headers)?,
        body: RequestBody::Empty,
        deadline,
    })
}

// LiteLLM appends the token as text, so an existing query string keeps every byte before the separator
fn page_url(base: &str, page_token: Option<&str>) -> String {
    page_token.filter(|token| !token.is_empty()).map_or_else(
        || base.to_owned(),
        |token| {
            let separator = if base.contains('?') { '&' } else { '?' };
            format!("{base}{separator}pageToken={token}")
        },
    )
}

pub fn cache_headers(
    target: &EndpointTarget,
    extra_headers: &BTreeMap<String, String>,
) -> Result<HeaderMap, CacheLookupError> {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    match &target.auth {
        None => {}
        Some(AuthHeader::Token(token)) => {
            let mut value = header_value(&format!("Bearer {}", token.expose_secret()))?;
            value.set_sensitive(true);
            headers.insert(AUTHORIZATION, value);
        }
        Some(AuthHeader::Headers(entries)) => {
            for (name, secret) in entries {
                let header = header_name(name)?;
                let mut value = header_value(secret.expose_secret())?;
                value.set_sensitive(true);
                headers.insert(header, value);
            }
        }
    }
    for (name, value) in extra_headers {
        headers.insert(header_name(name)?, header_value(value)?);
    }
    Ok(headers)
}

fn header_name(name: &str) -> Result<HeaderName, CacheLookupError> {
    HeaderName::from_bytes(name.as_bytes()).map_err(|source| CacheLookupError::Header {
        source: http::Error::from(source),
    })
}

fn header_value(value: &str) -> Result<HeaderValue, CacheLookupError> {
    HeaderValue::from_str(value).map_err(|source| CacheLookupError::Header {
        source: http::Error::from(source),
    })
}
