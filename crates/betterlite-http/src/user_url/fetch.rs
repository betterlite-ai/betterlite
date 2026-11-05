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
    validator: UrlValidator<R>,
    validation: bool,
}

fn get_request(
    client: &ProxiedClient,
    url: &str,
    headers: HeaderMap,
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
        deadline: client.limits.read_timeout,
    })
}

impl<R> UserUrlFetcher<R>
where
    R: Resolve,
{
    pub const fn new(validator: UrlValidator<R>, validation: bool) -> Self {
        Self {
            validator,
            validation,
        }
    }

    // LiteLLM safe_get and async_safe_get: the response comes back with its body unread
    pub async fn fetch(
        &self,
        client: &ProxiedClient,
        url: &str,
        headers: &HeaderMap,
    ) -> Result<StreamResponse, UserUrlError> {
        if self.validation {
            self.fetch_validated(client, url, headers).await
        } else {
            Self::fetch_unvalidated(client, url, headers).await
        }
    }

    async fn fetch_validated(
        &self,
        client: &ProxiedClient,
        url: &str,
        headers: &HeaderMap,
    ) -> Result<StreamResponse, UserUrlError> {
        let mut current = url.to_owned();
        for _ in 0..MAX_VALIDATED_REDIRECTS {
            let validated = self.validator.validate_url(&current).await?;
            let mut hop_headers = headers.clone();
            let host = HeaderValue::try_from(validated.host_header).map_err(|source| {
                TransportError::InvalidHeader {
                    name: "host".to_owned(),
                    source,
                }
            })?;
            hop_headers.insert(HOST, host);
            let response = client
                .send(get_request(client, &validated.url, hop_headers)?)
                .await?;
            if !response.response.status().is_redirection() {
                return Ok(response);
            }
            let target = location(response.response.headers())
                .filter(|text| !text.is_empty())
                .ok_or(SsrfError::NoLocation)?;
            current = join(&HttpxUrl::parse(&current)?, &target)?.href();
        }
        Err(UserUrlError::Ssrf(SsrfError::TooManyRedirects))
    }

    async fn fetch_unvalidated(
        client: &ProxiedClient,
        url: &str,
        headers: &HeaderMap,
    ) -> Result<StreamResponse, UserUrlError> {
        let mut current = HttpxUrl::parse(url)?;
        let mut current_headers = headers.clone();
        for _ in 0..=MAX_UNVALIDATED_REDIRECTS {
            let response = client
                .send(get_request(
                    client,
                    &current.href(),
                    current_headers.clone(),
                )?)
                .await?;
            if !has_redirect_location(&response.response) {
                return Ok(response);
            }
            let target = location(response.response.headers()).unwrap_or_default();
            let next = redirect_url(&current, &target)?;
            current_headers = redirect_headers(&current_headers, &current, &next);
            current = next;
        }
        Err(UserUrlError::TooManyRedirects)
    }
}
