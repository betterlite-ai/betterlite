use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_util::stream::BoxStream;
use futures_util::{Stream, StreamExt as _};
use sse_stream::{Error as SseError, Sse, SseStream};

use crate::client::cleanup::recording::ResponseBody as _;
use crate::client::http_transport::error::McpHttpError;
use crate::client::http_transport::send::HttpBody;

use super::error::SdkError;

pub(super) const MAX_EVENT_SIZE: usize = 1024 * 1024;

pub(super) struct EventStream {
    inner: BoxStream<'static, Result<Sse, SseError>>,
}
impl EventStream {
    pub(super) fn open(body: HttpBody, limit: usize) -> Self {
        let chunks = futures_util::stream::unfold(body, |mut body| async move {
            let chunk = body.next_chunk().await?;
            Some((chunk.map(Bytes::from), body))
        });
        let bounded = Bounded {
            inner: chunks.boxed(),
            limiter: Limiter::new(limit),
            failed: false,
        };
        Self {
            inner: SseStream::from_bytes_stream(bounded).boxed(),
        }
    }
    pub(super) async fn next(&mut self) -> Option<Result<Sse, SdkError>> {
        match self.inner.next().await {
            Some(event) => Some(event.map_err(sdk_error)),
            None => None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum LimitError {
    #[error("the SSE source failed")]
    Source(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("the SSE event is larger than {limit} bytes")]
    TooLarge { limit: usize },
}
impl LimitError {
    fn into_sdk(self) -> SdkError {
        match self {
            Self::TooLarge { limit } => SdkError::EventTooLarge { limit },
            Self::Source(source) => match source.downcast::<McpHttpError>() {
                Ok(error) => SdkError::Http(Arc::new(*error)),
                Err(other) => SdkError::Sse(SseError::Body(other)),
            },
        }
    }
}
fn sdk_error(error: SseError) -> SdkError {
    match error {
        SseError::Body(source) => match source.downcast::<LimitError>() {
            Ok(limit) => limit.into_sdk(),
            Err(other) => SdkError::Sse(SseError::Body(other)),
        },
        framing => SdkError::Sse(framing),
    }
}
struct Limiter {
    maximum: usize,
    retained: usize,
    blank: bool,
}
struct OverLimit;
impl Limiter {
    const fn new(maximum: usize) -> Self {
        Self {
            maximum,
            retained: 0,
            blank: false,
        }
    }
    fn observe(&mut self, chunk: &[u8]) -> Result<(), OverLimit> {
        for &byte in chunk {
            match byte {
                b'\n' => {
                    if self.blank {
                        self.retained = 0;
                    }
                    self.blank = !self.blank;
                }
                b'\r' => {}
                _ => {
                    self.blank = false;
                    self.retained = self.retained.saturating_add(1);
                    if self.retained > self.maximum {
                        return Err(OverLimit);
                    }
                }
            }
        }
        Ok(())
    }
}
struct Bounded {
    inner: BoxStream<'static, Result<Bytes, McpHttpError>>,
    limiter: Limiter,
    failed: bool,
}
impl Stream for Bounded {
    type Item = Result<Bytes, LimitError>;
    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.failed {
            return Poll::Ready(None);
        }
        match self.inner.as_mut().poll_next(context) {
            Poll::Ready(Some(Ok(chunk))) => {
                if self.limiter.observe(&chunk).is_err() {
                    self.failed = true;
                    return Poll::Ready(Some(Err(LimitError::TooLarge {
                        limit: self.limiter.maximum,
                    })));
                }
                Poll::Ready(Some(Ok(chunk)))
            }
            Poll::Ready(Some(Err(error))) => {
                self.failed = true;
                Poll::Ready(Some(Err(LimitError::Source(Box::new(error)))))
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}
