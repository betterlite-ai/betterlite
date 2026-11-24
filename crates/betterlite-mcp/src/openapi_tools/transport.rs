use std::collections::BTreeMap;
use std::future::Future;
use std::mem;
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
    CONTENT_LENGTH, HeaderMap, HeaderName, HeaderValue, InvalidHeaderName, InvalidHeaderValue,
    TRANSFER_ENCODING, ToStrError,
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
        source: InvalidHeaderName,
    },
    #[error("the {name} header holds a character a header cannot carry")]
    HeaderValue {
        name: String,
        #[source]
        source: InvalidHeaderValue,
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

pub trait OpenApiTransport: Send + Sync {
    fn send<'call>(
        &'call self,
        request: OpenApiRequest,
    ) -> Pin<Box<dyn Future<Output = Result<UpstreamResponse, OpenApiTransportError>> + Send + 'call>>;
}

// the pooled client never follows redirects itself, so a tool call hands it this transport instead
pub struct ProxiedTransport(pub Arc<ProxiedClient>);

impl OpenApiTransport for ProxiedTransport {
    fn send<'call>(
        &'call self,
        request: OpenApiRequest,
    ) -> Pin<Box<dyn Future<Output = Result<UpstreamResponse, OpenApiTransportError>> + Send + 'call>>
    {
        Box::pin(follow(&self.0, request))
    }
}

// the method, url, headers, body and query a redirect chain carries from one hop to the next
struct Hop {
    method: Method,
    current: HttpxUrl,
    headers: BTreeMap<String, String>,
    body: Option<Vec<u8>>,
    query: Vec<(String, String)>,
}

impl Hop {
    fn outbound(
        &mut self,
        sent: &HeaderMap,
        deadline: Duration,
    ) -> Result<OutboundRequest, OpenApiTransportError> {
        Ok(OutboundRequest {
            method: self.method.clone(),
            url: hop_url(&self.current)?,
            query: mem::take(&mut self.query),
            headers: sent.clone(),
            body: self.body.as_ref().map_or_else(
                || RequestBody::Empty,
                |bytes| RequestBody::Json(bytes.clone()),
            ),
            deadline,
        })
    }

    fn advance(
        &mut self,
        status: u16,
        sent: &HeaderMap,
        target: &str,
    ) -> Result<(), OpenApiTransportError> {
        let next = redirect_url(&self.current, target)
            .map_err(|source| failed_url(&self.current.href(), source))?;
        let next_method = redirect_method(status, &self.method);
        let mut next_headers = redirect_headers(sent, &self.current, &next);
        if next_method != self.method {
            next_headers.remove(CONTENT_LENGTH);
            next_headers.remove(TRANSFER_ENCODING);
            self.body = None;
        }
        let mut headers = BTreeMap::new();
        for (name, value) in &next_headers {
            let text = value
                .to_str()
                .map_err(|source| OpenApiTransportError::HeaderText {
                    name: name.as_str().to_owned(),
                    source,
                })?;
            headers.insert(name.as_str().to_owned(), text.to_owned());
        }
        self.headers = headers;
        self.method = next_method;
        self.current = next;
        Ok(())
    }
}

// httpx follows redirects on its shared clients; the pooled client here never does, so each hop is built
async fn follow(
    client: &ProxiedClient,
    request: OpenApiRequest,
) -> Result<UpstreamResponse, OpenApiTransportError> {
    let mut hop = Hop {
        method: request.method,
        current: HttpxUrl::parse(&request.url)
            .map_err(|source| failed_url(&request.url, UserUrlError::from(source)))?,
        headers: request.headers,
        body: request.body,
        query: request.query,
    };
    for _ in 0..=MAX_REDIRECTS {
        if let Some(guard) = &request.redirect_guard {
            guard.apply(&hop.current.href(), &mut hop.headers);
        }
        let sent = header_map(&hop.headers)?;
        let outbound = hop.outbound(&sent, client.limits.read_timeout)?;
        let response = client.send_buffered(outbound).await?;
        if !matches!(response.status, 301 | 302 | 303 | 307 | 308) {
            return Ok(response);
        }
        let Some(target) = location(&response.headers) else {
            return Ok(response);
        };
        hop.advance(response.status, &sent, &target)?;
    }
    Err(OpenApiTransportError::Send(
        TransportError::TooManyRedirects {
            limit: MAX_REDIRECTS,
        },
    ))
}

fn failed_url(url: &str, source: UserUrlError) -> OpenApiTransportError {
    OpenApiTransportError::Url {
        url: url.to_owned(),
        source,
    }
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
