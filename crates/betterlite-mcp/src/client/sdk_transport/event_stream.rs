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
    Source(#[source] McpHttpError),
    #[error("the SSE event is larger than {limit} bytes")]
    TooLarge { limit: usize },
}
fn sdk_error(error: SseError) -> SdkError {
    match error {
        SseError::Body(source) => match source.downcast::<LimitError>() {
            Ok(limit_error) => match *limit_error {
                LimitError::TooLarge { limit } => SdkError::EventTooLarge { limit },
                LimitError::Source(source) => SdkError::Http(Arc::new(source)),
            },
            Err(other) => SdkError::Sse(SseError::Body(other)),
        },
        framing => SdkError::Sse(framing),
    }
}
struct Limiter {
    maximum: usize,
    retained: usize,
    line: usize,
    comment: bool,
    after_cr: bool,
}
struct OverLimit;

// httpx2 ends a line on CR, CRLF or LF and leaves comment lines out of the event size
impl Limiter {
    const fn new(maximum: usize) -> Self {
        Self {
            maximum,
            retained: 0,
            line: 0,
            comment: false,
            after_cr: false,
        }
    }
    fn observe(&mut self, chunk: &[u8]) -> Result<(), OverLimit> {
        for &byte in chunk {
            if self.after_cr {
                self.after_cr = false;
                if byte == b'\n' {
                    continue;
                }
            }
            match byte {
                b'\r' => {
                    self.finish_line()?;
                    self.after_cr = true;
                }
                b'\n' => self.finish_line()?,
                _ => {
                    if self.line == 0 {
                        self.comment = byte == b':';
                    }
                    self.line = self.line.saturating_add(1);
                    if self.retained.saturating_add(self.line) > self.maximum {
                        return Err(OverLimit);
                    }
                }
            }
        }
        Ok(())
    }
    fn finish_line(&mut self) -> Result<(), OverLimit> {
        if self.line == 0 {
            self.retained = 0;
        } else if !self.comment {
            self.retained = self.retained.saturating_add(self.line);
        }
        self.line = 0;
        self.comment = false;
        if self.retained.saturating_add(self.line) > self.maximum {
            return Err(OverLimit);
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
                Poll::Ready(Some(Err(LimitError::Source(error))))
            }
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}
