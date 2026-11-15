use std::collections::BTreeMap;
use std::future::Future;
use std::mem;
use std::pin::Pin;
use std::time::Duration;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use futures_util::stream::BoxStream;
use http::Method;
use http::header::{HeaderMap, HeaderName, HeaderValue};
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use crate::vertex_embedding_call::post::{ClientTimeout, PostError};

pub type PostFuture<'request> =
    Pin<Box<dyn Future<Output = Result<PostReply, PostError>> + Send + 'request>>;

pub type OpenFuture<'request> =
    Pin<Box<dyn Future<Output = Result<OpenedReply, PostError>> + Send + 'request>>;

// the body of a streamed response as the pool hands it out
pub type Chunks = BoxStream<'static, Result<Vec<u8>, HttpError>>;

pub struct PostRequest<'request> {
    pub url: &'request str,
    pub headers: &'request BTreeMap<String, SecretString>,
    pub body: &'request [u8],
    pub timeout: ClientTimeout,
}

pub struct PostReply {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

pub struct OpenedReply {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: Chunks,
}

// LiteLLM's httpx handlers: post waits for the whole answer, post(..., stream=True) returns at the head
pub trait GeminiPost: Send + Sync {
    fn post<'request>(&'request self, request: PostRequest<'request>) -> PostFuture<'request>;

    fn open<'request>(&'request self, request: PostRequest<'request>) -> OpenFuture<'request>;
}

impl GeminiPost for ProxiedClient {
    fn post<'request>(&'request self, request: PostRequest<'request>) -> PostFuture<'request> {
        Box::pin(async move { buffered(self, &request).await })
    }

    fn open<'request>(&'request self, request: PostRequest<'request>) -> OpenFuture<'request> {
        Box::pin(async move {
            let mut response = self
                .send_following(build_outbound(&request, self.limits.read_timeout)?)
                .await
                .map_err(send_error)?;
            let headers = mem::take(response.response.headers_mut());
            let status = response.response.status().as_u16();
            Ok(OpenedReply {
                status,
                headers,
                body: response.body_stream(),
            })
        })
    }
}

async fn buffered(
    client: &ProxiedClient,
    request: &PostRequest<'_>,
) -> Result<PostReply, PostError> {
    let response = client
        .send_following(build_outbound(request, client.limits.read_timeout)?)
        .await
        .map_err(send_error)?
        .buffered(client.limits.body_bytes)
        .await
        .map_err(send_error)?;
    Ok(PostReply {
        status: response.status,
        headers: response.headers,
        body: response.body,
    })
}

fn build_outbound(
    request: &PostRequest<'_>,
    handler_timeout: Duration,
) -> Result<OutboundRequest, PostError> {
    let url = Url::parse(request.url).map_err(PostError::Url)?;
    let headers = wire_headers(request.headers)?;
    let deadline = match request.timeout {
        ClientTimeout::Explicit(timeout) => timeout.total,
        ClientTimeout::HandlerDefault => handler_timeout,
    };
    Ok(OutboundRequest {
        method: Method::POST,
        url,
        query: Vec::new(),
        headers,
        body: RequestBody::Json(request.body.to_vec()),
        deadline,
    })
}

fn wire_headers(headers: &BTreeMap<String, SecretString>) -> Result<HeaderMap, PostError> {
    let mut map = HeaderMap::new();
    for (name, secret) in headers {
        let header =
            HeaderName::from_bytes(name.as_bytes()).map_err(|source| PostError::HeaderName {
                name: name.clone(),
                source,
            })?;
        let mut value = HeaderValue::from_str(secret.expose_secret()).map_err(|source| {
            PostError::HeaderValue {
                name: name.clone(),
                source,
            }
        })?;
        // a Gemini key rides x-goog-api-key, which the pool's own credential list does not cover
        let credential = matches!(
            header.as_str(),
            "authorization" | "proxy-authorization" | "x-api-key" | "api-key" | "x-goog-api-key"
        );
        if credential {
            value.set_sensitive(true);
        }
        map.insert(header, value);
    }
    Ok(map)
}

fn send_error(source: TransportError) -> PostError {
    match source {
        TransportError::ConnectTimeout(_) | TransportError::Deadline(_) => PostError::Timeout,
        source => PostError::Send { source },
    }
}
