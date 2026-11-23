use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use betterlite_http::user_url::error::UserUrlError;
use betterlite_http::user_url::httpx_url::HttpxUrl;
use betterlite_http::user_url::redirect::{location, redirect_headers, redirect_url};
use http::Method;
use http::header::{
    CONTENT_LENGTH, HeaderMap, HeaderName, HeaderValue, TRANSFER_ENCODING, ToStrError,
};
use thiserror::Error;
use url::Url;

use crate::client::sessions::factory::RedirectGuard;

// httpx's default max_redirects: the initial request plus up to twenty followed redirects
pub const MAX_REDIRECTS: usize = 20;

#[derive(Debug, Clone)]
pub struct OpenApiRequest {
    pub method: Method,
    pub url: String,
    pub query: Vec<(String, String)>,
    pub headers: BTreeMap<String, String>,
    pub body: Option<Vec<u8>>,
    pub redirect_guard: Option<RedirectGuard>,
}

#[derive(Debug, Error)]
pub enum OpenApiTransportError {
    #[error("the request url {url:?} cannot be requested")]
    Url {
        url: String,
        #[source]
        source: UserUrlError,
    },
    #[error("the {name} header is not a valid header name")]
    HeaderName {
        name: String,
        #[source]
        source: http::header::InvalidHeaderName,
    },
    #[error("the {name} header holds a character a header cannot carry")]
    HeaderValue {
        name: String,
        #[source]
        source: http::header::InvalidHeaderValue,
    },
    #[error("the {name} header cannot be read as text")]
    HeaderText {
        name: String,
        #[source]
        source: ToStrError,
    },
    #[error(transparent)]
    Send(#[from] TransportError),
}

pub trait OpenApiTransport {
    fn send<'a>(
        &'a self,
        request: OpenApiRequest,
    ) -> Pin<Box<dyn Future<Output = Result<UpstreamResponse, OpenApiTransportError>> + Send + 'a>>;
}

pub struct ProxiedTransport {
    client: Arc<ProxiedClient>,
    timeout: Duration,
}

impl ProxiedTransport {
    #[must_use]
    pub fn new(client: Arc<ProxiedClient>) -> Self {
        let timeout = client.limits.read_timeout;
        Self { client, timeout }
    }
}

impl OpenApiTransport for ProxiedTransport {
    fn send<'a>(
        &'a self,
        request: OpenApiRequest,
    ) -> Pin<Box<dyn Future<Output = Result<UpstreamResponse, OpenApiTransportError>> + Send + 'a>>
    {
        Box::pin(self.follow(request))
    }
}

impl ProxiedTransport {
    // httpx follows redirects on its shared clients; the pooled client here never does, so each hop is built
    async fn follow(
        &self,
        request: OpenApiRequest,
    ) -> Result<UpstreamResponse, OpenApiTransportError> {
        let mut method = request.method;
        let mut current = HttpxUrl::parse(&request.url)
            .map_err(|source| failed_url(&request.url, UserUrlError::from(source)))?;
        let mut headers = request.headers;
        let mut body = request.body;
        let mut query = request.query;
        for _ in 0..=MAX_REDIRECTS {
            if let Some(guard) = &request.redirect_guard {
                guard.apply(&current.href(), &mut headers);
            }
            let sent = header_map(&headers)?;
            let outbound = OutboundRequest {
                method: method.clone(),
                url: hop_url(&current)?,
                query: std::mem::take(&mut query),
                headers: sent.clone(),
                body: match &body {
                    Some(bytes) => RequestBody::Json(bytes.clone()),
                    None => RequestBody::Empty,
                },
                deadline: self.timeout,
            };
            let response = self.client.send_buffered(outbound).await?;
            if !is_redirect(response.status) {
                return Ok(response);
            }
            let Some(target) = location(&response.headers) else {
                return Ok(response);
            };
            let next = redirect_url(&current, &target)
                .map_err(|source| failed_url(&current.href(), source))?;
            let next_method = redirect_method(response.status, &method);
            let mut next_headers = redirect_headers(&sent, &current, &next);
            if next_method != method {
                next_headers.remove(CONTENT_LENGTH);
                next_headers.remove(TRANSFER_ENCODING);
                body = None;
            }
            headers = from_header_map(&next_headers)?;
            method = next_method;
            current = next;
        }
        Err(OpenApiTransportError::Send(
            TransportError::TooManyRedirects {
                limit: MAX_REDIRECTS,
            },
        ))
    }
}

fn failed_url(url: &str, source: UserUrlError) -> OpenApiTransportError {
    OpenApiTransportError::Url {
        url: url.to_owned(),
        source,
    }
}

fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

fn hop_url(current: &HttpxUrl) -> Result<Url, OpenApiTransportError> {
    let text = current.href();
    Url::parse(&text).map_err(|source| {
        failed_url(
            &text,
            UserUrlError::Unrequestable {
                url: text.clone(),
                source,
            },
        )
    })
}

fn header_map(headers: &BTreeMap<String, String>) -> Result<HeaderMap, OpenApiTransportError> {
    let mut map = HeaderMap::with_capacity(headers.len());
    for (name, value) in headers {
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|source| {
            OpenApiTransportError::HeaderName {
                name: name.clone(),
                source,
            }
        })?;
        let header_value =
            HeaderValue::from_str(value).map_err(|source| OpenApiTransportError::HeaderValue {
                name: name.clone(),
                source,
            })?;
        map.insert(header_name, header_value);
    }
    Ok(map)
}

fn from_header_map(map: &HeaderMap) -> Result<BTreeMap<String, String>, OpenApiTransportError> {
    let mut headers = BTreeMap::new();
    for (name, value) in map {
        let text = value
            .to_str()
            .map_err(|source| OpenApiTransportError::HeaderText {
                name: name.as_str().to_owned(),
                source,
            })?;
        headers.insert(name.as_str().to_owned(), text.to_owned());
    }
    Ok(headers)
}

// httpx Client._redirect_method
fn redirect_method(status: u16, method: &Method) -> Method {
    let gets_rewritten = status == 303 || status == 302;
    if gets_rewritten && *method != Method::HEAD {
        return Method::GET;
    }
    if status == 301 && *method == Method::POST {
        return Method::GET;
    }
    method.clone()
}
