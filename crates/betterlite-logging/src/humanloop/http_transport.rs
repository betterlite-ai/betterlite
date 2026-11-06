use std::future::Future;
use std::string::FromUtf8Error;
use std::sync::Arc;
use std::time::{Duration, Instant};

use betterlite_config::proxy::ProxyPolicy;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse, base_builder};
use http::Method;
use http::header::{
    HeaderMap, HeaderName, HeaderValue, InvalidHeaderName, InvalidHeaderValue, LOCATION, ToStrError,
};
use secrecy::{ExposeSecret as _, SecretString};
use url::{ParseError, Url};

use crate::humanloop::transport::{PromptFetch, PromptTransport, TransportResponse};

pub const PROMPT_DEADLINE: Duration = Duration::from_secs(600);
pub const PROMPT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub const MAX_PROMPT_BODY_BYTES: usize = 0x0010_0000;
pub const MAX_PROMPT_REDIRECTS: usize = 20;
const PROMPT_HEADER_BYTES: usize = 0x0001_0000;
const PROMPT_LIMITS: TransportLimits = TransportLimits {
    connect_timeout: PROMPT_CONNECT_TIMEOUT,
    read_timeout: PROMPT_DEADLINE,
    body_bytes: MAX_PROMPT_BODY_BYTES,
    header_bytes: PROMPT_HEADER_BYTES,
};
const REDIRECT_STATUSES: [u16; 5] = [301, 302, 303, 307, 308];

#[derive(Debug, thiserror::Error)]
pub enum HttpTransportError {
    #[error("the Humanloop prompt URL is not valid")]
    Url(#[source] ParseError),
    #[error("the Humanloop request header name is not valid")]
    HeaderName(#[source] InvalidHeaderName),
    #[error("the Humanloop request header value is not valid")]
    HeaderValue(#[source] InvalidHeaderValue),
    #[error("the Humanloop redirect target is not valid")]
    Redirect(#[source] ParseError),
    #[error("the Humanloop redirect location is not valid text")]
    RedirectLocation(#[source] ToStrError),
    #[error("the Humanloop redirect limit of {limit} was exceeded")]
    TooManyRedirects { limit: usize },
    #[error("the Humanloop request did not finish within {seconds} seconds")]
    Deadline { seconds: u64 },
    #[error("the request to Humanloop failed")]
    Transport(#[source] TransportError),
    #[error("the Humanloop response body is not UTF-8")]
    BodyUtf8(#[source] FromUtf8Error),
}

#[derive(Debug)]
pub struct HttpPromptTransport {
    pub client: ProxiedClient,
    pub deadline: Duration,
}

impl HttpPromptTransport {
    pub fn new(policy: &Arc<ProxyPolicy>) -> Result<Self, HttpError> {
        Ok(Self {
            client: ProxiedClient::build(base_builder(), policy, PROMPT_LIMITS)?,
            deadline: PROMPT_DEADLINE,
        })
    }

    async fn send(&self, fetch: &PromptFetch) -> Result<TransportResponse, HttpTransportError> {
        let started = Instant::now();
        let mut url = Url::parse(&fetch.url).map_err(HttpTransportError::Url)?;
        let headers = header_map(&fetch.headers)?;
        for _attempt in 0..=MAX_PROMPT_REDIRECTS {
            let remaining = self
                .deadline
                .checked_sub(started.elapsed())
                .filter(|remaining| !remaining.is_zero())
                .ok_or(HttpTransportError::Deadline {
                    seconds: self.deadline.as_secs(),
                })?;
            let response = self.single_hop(&url, &headers, remaining).await?;
            if REDIRECT_STATUSES.contains(&response.status)
                && let Some(raw_location) = response.headers.get(LOCATION)
            {
                let location = raw_location
                    .to_str()
                    .map_err(HttpTransportError::RedirectLocation)?;
                url = url.join(location).map_err(HttpTransportError::Redirect)?;
                continue;
            }
            return Ok(TransportResponse {
                status: response.status,
                body: String::from_utf8(response.body).map_err(HttpTransportError::BodyUtf8)?,
            });
        }
        Err(HttpTransportError::TooManyRedirects {
            limit: MAX_PROMPT_REDIRECTS,
        })
    }

    async fn single_hop(
        &self,
        url: &Url,
        headers: &HeaderMap,
        remaining: Duration,
    ) -> Result<UpstreamResponse, HttpTransportError> {
        let request = OutboundRequest {
            method: Method::GET,
            url: url.clone(),
            query: Vec::new(),
            headers: headers.clone(),
            body: RequestBody::Empty,
            deadline: remaining,
        };
        self.client
            .send_buffered(request)
            .await
            .map_err(HttpTransportError::Transport)
    }
}

fn header_map(headers: &[(&'static str, SecretString)]) -> Result<HeaderMap, HttpTransportError> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let header_name =
            HeaderName::from_bytes(name.as_bytes()).map_err(HttpTransportError::HeaderName)?;
        let header_value = HeaderValue::try_from(value.expose_secret())
            .map_err(HttpTransportError::HeaderValue)?;
        map.insert(header_name, header_value);
    }
    Ok(map)
}

impl PromptTransport for HttpPromptTransport {
    type Error = HttpTransportError;

    fn get(
        &self,
        fetch: &PromptFetch,
    ) -> impl Future<Output = Result<TransportResponse, HttpTransportError>> + Send {
        self.send(fetch)
    }
}
