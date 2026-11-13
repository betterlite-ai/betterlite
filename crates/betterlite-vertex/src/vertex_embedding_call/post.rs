use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use http::Method;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;
use url::Url;

pub type PostFuture<'request> =
    Pin<Box<dyn Future<Output = Result<PostReply, PostError>> + Send + 'request>>;

// httpx.Timeout as a handler is built with it: every phase runs no longer than total unless connect is set apart
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Timeout {
    pub total: Duration,
    pub connect: Duration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClientTimeout {
    Explicit(Timeout),
    // No timeout reached the handler, so the handler's own configured timeout governs the call
    HandlerDefault,
}

// LiteLLM's _DEFAULT_TIMEOUT: COMPLETION_HTTP_FALLBACK_SECONDS with a five-second connect phase
pub const HANDLER_DEFAULT_TIMEOUT: Timeout = Timeout {
    total: Duration::from_secs(600),
    connect: Duration::from_secs(5),
};

pub struct PostRequest<'request> {
    pub url: &'request str,
    pub headers: &'request BTreeMap<String, SecretString>,
    pub body: &'request [u8],
    pub timeout: ClientTimeout,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PostReply {
    pub status: u16,
    pub body: Vec<u8>,
}

pub trait EmbeddingPost: Send + Sync {
    fn post<'request>(&'request self, request: PostRequest<'request>) -> PostFuture<'request>;
}

#[derive(Debug, Error)]
pub enum PostError {
    #[error("the request URL is not valid: {source}")]
    Url(#[source] url::ParseError),
    #[error("the {name} header name is not valid: {source}")]
    HeaderName {
        name: String,
        #[source]
        source: http::header::InvalidHeaderName,
    },
    #[error("the {name} header value is not valid: {source}")]
    HeaderValue {
        name: String,
        #[source]
        source: http::header::InvalidHeaderValue,
    },
    #[error("the request timed out")]
    Timeout,
    #[error("the request could not be sent: {source}")]
    Send {
        #[source]
        source: TransportError,
    },
}

impl EmbeddingPost for ProxiedClient {
    fn post<'request>(&'request self, request: PostRequest<'request>) -> PostFuture<'request> {
        Box::pin(async move {
            let url = Url::parse(request.url).map_err(PostError::Url)?;
            let headers = header_map(request.headers)?;
            let deadline = match request.timeout {
                ClientTimeout::Explicit(timeout) => timeout.total,
                ClientTimeout::HandlerDefault => self.limits.read_timeout,
            };
            let outbound = OutboundRequest {
                method: Method::POST,
                url,
                query: Vec::new(),
                headers,
                body: RequestBody::Json(request.body.to_vec()),
                deadline,
            };
            match self.send_buffered(outbound).await {
                Ok(response) => Ok(PostReply {
                    status: response.status,
                    body: response.body,
                }),
                Err(TransportError::ConnectTimeout(_) | TransportError::Deadline(_)) => {
                    Err(PostError::Timeout)
                }
                Err(source) => Err(PostError::Send { source }),
            }
        })
    }
}

fn header_map(headers: &BTreeMap<String, SecretString>) -> Result<HeaderMap, PostError> {
    let mut map = HeaderMap::with_capacity(headers.len());
    for (name, value) in headers {
        let header_name =
            HeaderName::from_bytes(name.as_bytes()).map_err(|source| PostError::HeaderName {
                name: name.clone(),
                source,
            })?;
        let mut header_value = HeaderValue::from_str(value.expose_secret()).map_err(|source| {
            PostError::HeaderValue {
                name: name.clone(),
                source,
            }
        })?;
        header_value.set_sensitive(true);
        map.insert(header_name, header_value);
    }
    Ok(map)
}
