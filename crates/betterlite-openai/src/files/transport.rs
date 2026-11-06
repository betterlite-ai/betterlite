use std::future::Future;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::proxy::{ProxiedClient, UpstreamResponse};
use futures_util::stream::BoxStream;
use http::HeaderMap;

pub struct StreamReply {
    pub status: u16,
    pub headers: HeaderMap,
    pub body: BoxStream<'static, Result<Vec<u8>, HttpError>>,
}

// B173's pooled client in production, a recorded reply in tests and in the oracle helper
pub trait FileTransport: Sync {
    fn send_buffered(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send;

    fn send_stream(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<StreamReply, TransportError>> + Send;
}

impl FileTransport for ProxiedClient {
    async fn send_buffered(
        &self,
        request: OutboundRequest,
    ) -> Result<UpstreamResponse, TransportError> {
        Self::send_buffered(self, request).await
    }

    async fn send_stream(&self, request: OutboundRequest) -> Result<StreamReply, TransportError> {
        let response = self.send(request).await?;
        Ok(StreamReply {
            status: response.response.status().as_u16(),
            headers: response.response.headers().clone(),
            body: response.body_stream(),
        })
    }
}
