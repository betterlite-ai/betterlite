use async_trait::async_trait;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::assistants::client::ClientKind;
use futures_util::stream::BoxStream;
use thiserror::Error;

pub type BytezChunks = BoxStream<'static, Result<Vec<u8>, HttpError>>;

pub struct StreamHead {
    pub status: u16,
    pub chunks: BytezChunks,
}

#[derive(Debug, Error)]
pub enum ClientError {
    // httpx.HTTPStatusError: a client that raises on a status instead of returning the response
    #[error("the Bytez request failed with status {status_code}: {message}")]
    Status { status_code: u16, message: String },
    #[error("the Bytez request failed")]
    Transport {
        #[source]
        source: TransportError,
    },
}

#[async_trait]
pub trait BytezStreamTransport: Send + Sync {
    fn kind(&self) -> ClientKind;
    async fn open(&self, request: OutboundRequest) -> Result<StreamHead, ClientError>;
}

// betterlite has one pooled client and it reads asynchronously, so a sync call replaces it with its own default
#[async_trait]
impl BytezStreamTransport for ProxiedClient {
    fn kind(&self) -> ClientKind {
        ClientKind::Async
    }

    async fn open(&self, request: OutboundRequest) -> Result<StreamHead, ClientError> {
        let response = self
            .send_following(request)
            .await
            .map_err(|source| ClientError::Transport { source })?;
        let status = response.response.status().as_u16();
        Ok(StreamHead {
            status,
            chunks: response.body_stream(),
        })
    }
}
