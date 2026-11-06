use std::time::Duration;

use reqwest::header::{HOST, HeaderMap, HeaderValue};
use reqwest::{Method, Url};

use crate::proxy::outbound::{OutboundRequest, RequestBody};
use crate::proxy::transport_error::TransportError;
use crate::proxy::{ProxiedClient, StreamResponse};
use crate::user_url::error::{SsrfError, UserUrlError};
use crate::user_url::httpx_url::HttpxUrl;
use crate::user_url::join::join;
use crate::user_url::redirect::{has_redirect_location, location, redirect_headers, redirect_url};
use crate::user_url::validate::{Resolve, UrlValidator};

// LiteLLM's _MAX_REDIRECTS: validated requests, and httpx's default max_redirects for the unvalidated one
const MAX_VALIDATED_REDIRECTS: usize = 10;
const MAX_UNVALIDATED_REDIRECTS: usize = 20;

#[derive(Debug, Clone)]
pub struct UserUrlFetcher<R> {
    pub validator: UrlValidator<R>,
    pub validation: bool,
}

fn get_request(
    url: &str,
    headers: HeaderMap,
    deadline: Duration,
) -> Result<OutboundRequest, UserUrlError> {
    let parsed = Url::parse(url).map_err(|source| UserUrlError::Unrequestable {
        url: url.to_owned(),
        source,
    })?;
    Ok(OutboundRequest {
        method: Method::GET,
        url: parsed,
        query: Vec::new(),
        headers,
        body: RequestBody::Empty,
        deadline,
    })
}

// B173's client never follows redirects, so the unvalidated branch repeats httpx's rules here
async fn follow_redirects(
    client: &ProxiedClient,
    url: &str,
    headers: &HeaderMap,
) -> Result<StreamResponse, UserUrlError> {
    let mut current = (HttpxUrl::parse(url)?, headers.clone());
    for _ in 0..=MAX_UNVALIDATED_REDIRECTS {
        let (at, sent) = &current;
        let request = get_request(&at.href(), sent.clone(), client.limits.read_timeout)?;
        let response = client.send(request).await?;
        if !has_redirect_location(&response.response) {
            return Ok(response);
        }
        let target = location(response.response.headers()).unwrap_or_default();
        let next = redirect_url(at, &target)?;
        current = (next.clone(), redirect_headers(sent, at, &next));
    }
    Err(UserUrlError::TooManyRedirects)
}

impl<R> UserUrlFetcher<R>
where
    R: Resolve,
{
    // The connection goes to the validated IP, so Host carries the name the caller asked for
    async fn pinned_request(
        &self,
        url: &str,
        headers: &HeaderMap,
        deadline: Duration,
    ) -> Result<OutboundRequest, UserUrlError> {
        let validated = self.validator.validate_url(url).await?;
        let host = HeaderValue::try_from(validated.host_header).map_err(|source| {
            TransportError::InvalidHeader {
                name: "host".to_owned(),
                source,
            }
        })?;
        let mut request = get_request(&validated.url, headers.clone(), deadline)?;
        request.headers.insert(HOST, host);
        Ok(request)
    }

    // LiteLLM safe_get and async_safe_get: the response comes back with its body unread
    pub async fn fetch(
        &self,
        client: &ProxiedClient,
        url: &str,
        headers: &HeaderMap,
    ) -> Result<StreamResponse, UserUrlError> {
        if !self.validation {
            return follow_redirects(client, url, headers).await;
        }
        let mut current = url.to_owned();
        for _ in 0..MAX_VALIDATED_REDIRECTS {
            let request = self
                .pinned_request(&current, headers, client.limits.read_timeout)
                .await?;
            let response = client.send(request).await?;
            if !response.response.status().is_redirection() {
                return Ok(response);
            }
            let found = location(response.response.headers()).filter(|text| !text.is_empty());
            let target = found.ok_or(SsrfError::NoLocation)?;
            current = join(&HttpxUrl::parse(&current)?, &target)?.href();
        }
        Err(UserUrlError::Ssrf(SsrfError::TooManyRedirects))
    }
}
