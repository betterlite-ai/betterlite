use async_trait::async_trait;
use betterlite_http::client_kind::ClientKind;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use futures_util::stream::BoxStream;

pub type Chunks = BoxStream<'static, Result<Vec<u8>, HttpError>>;

pub struct StreamHead {
    pub status: u16,
    pub chunks: Chunks,
}

#[async_trait]
pub trait StreamTransport: Send + Sync {
    fn kind(&self) -> ClientKind;

    async fn post(&self, request: OutboundRequest) -> Result<StreamHead, TransportError>;
}

// betterlite reads asynchronously, so a sync call replaces the supplied client with its default
#[async_trait]
impl StreamTransport for ProxiedClient {
    fn kind(&self) -> ClientKind {
        ClientKind::Asynchronous
    }

    async fn post(&self, request: OutboundRequest) -> Result<StreamHead, TransportError> {
        let response = self.send_following(request).await?;
        Ok(StreamHead {
            status: response.response.status().as_u16(),
            chunks: response.body_stream(),
        })
    }
}
