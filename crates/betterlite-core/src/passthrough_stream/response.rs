use betterlite_http::error::HttpError;
use futures_util::StreamExt as _;
use futures_util::future::BoxFuture;
use futures_util::stream::BoxStream;
use futures_util::stream::empty;
use http::HeaderMap;

use super::error::PassthroughStreamError;

mod tests;

pub type PendingRelay = BoxFuture<'static, Result<AsyncResponse, PassthroughStreamError>>;

pub trait AsyncChunkSource: Send {
    fn next_chunk(&mut self) -> BoxFuture<'_, Result<Option<Vec<u8>>, HttpError>>;

    fn read_body(&mut self) -> BoxFuture<'_, Result<Vec<u8>, HttpError>>;

    fn close(&mut self) -> BoxFuture<'_, Result<(), HttpError>>;
}

pub trait SyncChunkSource: Send {
    fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, HttpError>;

    fn close(&mut self) -> Result<(), HttpError>;
}

pub struct AsyncResponse {
    pub status_code: u16,
    pub headers: HeaderMap,
    pub source: Box<dyn AsyncChunkSource>,
}

pub struct SyncResponse {
    pub status_code: u16,
    pub headers: HeaderMap,
    pub source: Box<dyn SyncChunkSource>,
}

pub struct StreamBody {
    pub chunks: BoxStream<'static, Result<Vec<u8>, HttpError>>,
    pub body_limit: usize,
}

impl AsyncChunkSource for StreamBody {
    fn next_chunk(&mut self) -> BoxFuture<'_, Result<Option<Vec<u8>>, HttpError>> {
        Box::pin(async move { self.chunks.next().await.transpose() })
    }

    fn read_body(&mut self) -> BoxFuture<'_, Result<Vec<u8>, HttpError>> {
        Box::pin(read_capped_body(&mut self.chunks, self.body_limit))
    }

    fn close(&mut self) -> BoxFuture<'_, Result<(), HttpError>> {
        Box::pin(async move {
            self.chunks = empty().boxed();
            Ok(())
        })
    }
}

async fn read_capped_body(
    chunks: &mut BoxStream<'static, Result<Vec<u8>, HttpError>>,
    body_limit: usize,
) -> Result<Vec<u8>, HttpError> {
    let mut body = Vec::new();
    while let Some(chunk) = chunks.next().await {
        let bytes = chunk?;
        let grown = body.len().checked_add(bytes.len());
        if grown.is_none_or(|total| total > body_limit) {
            return Err(HttpError::TooLarge { limit: body_limit });
        }
        body.extend_from_slice(&bytes);
    }
    Ok(body)
}
