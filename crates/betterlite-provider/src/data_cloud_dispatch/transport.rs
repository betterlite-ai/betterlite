use std::time::Duration;

use async_trait::async_trait;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::limits::TransportLimits;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_http::proxy::{ProxiedClient, StreamResponse, UpstreamResponse};
use futures_util::stream::BoxStream;
use http::{HeaderMap, Method};
use url::Url;

use crate::operation::OperationError;

use super::error::DispatchError;

pub struct WireRequest<'request> {
    pub method: Method,
    pub url: &'request str,
    pub headers: &'request HeaderMap,
    pub body: &'request [u8],
    pub deadline: Duration,
}

impl WireRequest<'_> {
    pub fn outbound(&self) -> Result<OutboundRequest, DispatchError> {
        Ok(OutboundRequest {
            method: self.method.clone(),
            url: Url::parse(self.url).map_err(|source| DispatchError::Url {
                url: self.url.to_owned(),
                source,
            })?,
            query: Vec::new(),
            headers: self.headers.clone(),
            body: RequestBody::Json(self.body.to_vec()),
            deadline: self.deadline,
        })
    }
}

pub struct StreamHead {
    pub status: u16,
    pub body: Vec<u8>,
    pub chunks: Option<BoxStream<'static, Result<Vec<u8>, HttpError>>>,
}

#[async_trait]
pub trait PostTransport: Send + Sync {
    async fn post(&self, request: &WireRequest<'_>) -> Result<UpstreamResponse, DispatchError>;
    async fn post_stream(&self, request: &WireRequest<'_>) -> Result<StreamHead, DispatchError>;
}

pub struct Pooled<'client> {
    pub client: &'client ProxiedClient,
}

#[async_trait]
impl PostTransport for Pooled<'_> {
    async fn post(&self, request: &WireRequest<'_>) -> Result<UpstreamResponse, DispatchError> {
        let response = self
            .client
            .send_following(request.outbound()?)
            .await
            .map_err(|source| DispatchError::Transport(OperationError::from(source)))?;
        buffered(response).await
    }

    async fn post_stream(&self, request: &WireRequest<'_>) -> Result<StreamHead, DispatchError> {
        let response = self
            .client
            .send_following(request.outbound()?)
            .await
            .map_err(|source| DispatchError::Transport(OperationError::from(source)))?;
        let status = response.response.status().as_u16();
        if status != 200 {
            let buffered = buffered(response).await?;
            return Ok(StreamHead {
                status,
                body: buffered.body,
                chunks: None,
            });
        }
        Ok(StreamHead {
            status,
            body: Vec::new(),
            chunks: Some(response.body_stream()),
        })
    }
}

async fn buffered(response: StreamResponse) -> Result<UpstreamResponse, DispatchError> {
    response
        .buffered(TransportLimits::DEFAULT.body_bytes)
        .await
        .map_err(|source| DispatchError::Transport(OperationError::from(source)))
}
