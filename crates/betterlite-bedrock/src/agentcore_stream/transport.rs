use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::transport_error::TransportError;
use futures_util::stream::BoxStream;
use http::{HeaderMap, Method};
use url::Url;

pub type Chunks = BoxStream<'static, Result<Vec<u8>, HttpError>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandlerKind {
    Sync,
    Async,
}

pub struct PostRequest {
    pub url: Url,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
    pub stream: bool,
    pub deadline: Duration,
}

pub struct StreamHead {
    pub status: u16,
    pub headers: HeaderMap,
    pub chunks: Chunks,
}

#[async_trait]
pub trait StreamHandler: Send + Sync {
    fn kind(&self) -> HandlerKind;

    async fn post(&self, request: PostRequest) -> Result<StreamHead, TransportError>;
}

// betterlite has one pooled client and it reads asynchronously, so a sync call replaces it with its default
#[async_trait]
impl StreamHandler for ProxiedClient {
    fn kind(&self) -> HandlerKind {
        HandlerKind::Async
    }

    async fn post(&self, request: PostRequest) -> Result<StreamHead, TransportError> {
        let response = self
            .send_following(OutboundRequest {
                method: Method::POST,
                url: request.url,
                query: Vec::new(),
                headers: request.headers,
                body: RequestBody::Json(request.body),
                deadline: request.deadline,
            })
            .await?;
        let status = response.response.status().as_u16();
        let headers = response.response.headers().clone();
        Ok(StreamHead {
            status,
            headers,
            chunks: response.body_stream(),
        })
    }
}
