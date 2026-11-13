use std::mem;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_util::stream::BoxStream;
use futures_util::{Stream, StreamExt as _};
use reqwest::Client;
use tokio_util::task::task_tracker::TaskTrackerToken;

use crate::error::HttpError;
use crate::proxy::limits::read_bounded;
use crate::proxy::transport_error::TransportError;
use crate::proxy::{StreamResponse, UpstreamResponse};

// Keeps the pooled client and the drain count alive for as long as the body can still be read.
pub struct Lease {
    pub(crate) _client: Client,
    pub(crate) _token: TaskTrackerToken,
}

struct LeasedStream {
    inner: BoxStream<'static, Result<Vec<u8>, HttpError>>,
    _lease: Lease,
}

impl Stream for LeasedStream {
    type Item = Result<Vec<u8>, HttpError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.poll_next_unpin(cx)
    }
}

impl StreamResponse {
    pub async fn buffered(mut self, limit: usize) -> Result<UpstreamResponse, TransportError> {
        let status = self.response.status().as_u16();
        let headers = mem::take(self.response.headers_mut());
        let body = read_bounded(&mut self.response, limit)
            .await
            .map_err(TransportError::reading)?;
        Ok(UpstreamResponse {
            status,
            headers,
            body,
        })
    }

    #[must_use]
    pub fn body_stream(self) -> BoxStream<'static, Result<Vec<u8>, HttpError>> {
        let inner = self
            .response
            .bytes_stream()
            .map(|chunk| {
                chunk
                    .map(|bytes| bytes.to_vec())
                    .map_err(HttpError::Request)
            })
            .boxed();
        LeasedStream {
            inner,
            _lease: self.lease,
        }
        .boxed()
    }
}

mod tests;
