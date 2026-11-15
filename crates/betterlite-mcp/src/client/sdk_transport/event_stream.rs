use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures_util::stream::{BoxStream, once, unfold};
use futures_util::{Stream, StreamExt as _};
use http::HeaderMap;
use http::header::CONTENT_TYPE;
use sse_stream::{Error as SseError, Sse, SseStream};

use crate::client::cleanup::recording::ResponseBody as _;
use crate::client::http_transport::error::McpHttpError;
use crate::client::http_transport::send::HttpBody;

use super::error::SdkError;
pub(super) const MAX_EVENT_SIZE: usize = 1024 * 1024;
pub(super) type EventStream = BoxStream<'static, Result<Sse, SdkError>>;
pub(super) fn content_type(headers: &HeaderMap) -> String {
    headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map_or_else(String::new, str::to_lowercase)
}
// httpx2 accepts only the event-stream media type itself, whatever its parameters
pub(super) fn is_sse_response(headers: &HeaderMap) -> bool {
    content_type(headers)
        .split(';')
        .next()
        .is_some_and(|kind| kind.trim() == "text/event-stream")
}
pub(super) fn open_stream(body: HttpBody, limit: usize, headers: &HeaderMap) -> EventStream {
    if !is_sse_response(headers) {
        let got = content_type(headers);
        return once(async move { Err(SdkError::ContentType { got }) }).boxed();
    }
    let chunks = unfold(body, |mut source| async move {
        let chunk = source.next_chunk().await?;
        Some((chunk.map(Bytes::from), source))
    });
    let bounded = Bounded {
        inner: chunks.boxed(),
        limiter: Limiter::new(limit),
        failed: false,
    };
    SseStream::from_bytes_stream(bounded)
        .map(|event| event.map_err(sdk_error))
        .boxed()
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
        SseError::Body(boxed) => match boxed.downcast::<LimitError>() {
            Ok(limit_error) => match *limit_error {
                LimitError::TooLarge { limit } => SdkError::EventTooLarge { limit },
                LimitError::Source(http) => SdkError::Http(Arc::new(http)),
            },
            Err(other) => SdkError::Sse(SseError::Body(other)),
        },
        framing @ (SseError::InvalidLine
        | SseError::DuplicatedEventLine
        | SseError::DuplicatedIdLine
        | SseError::DuplicatedRetry
        | SseError::Utf8Parse(_)
        | SseError::IntParse(_)) => SdkError::Sse(framing),
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
            self.step(byte)?;
        }
        Ok(())
    }
    fn step(&mut self, byte: u8) -> Result<(), OverLimit> {
        if self.after_cr {
            self.after_cr = false;
            if byte == b'\n' {
                return Ok(());
            }
        }
        match byte {
            b'\r' => {
                self.finish_line()?;
                self.after_cr = true;
                Ok(())
            }
            b'\n' => self.finish_line(),
            _ => self.count(byte),
        }
    }
    const fn count(&mut self, byte: u8) -> Result<(), OverLimit> {
        if self.line == 0 {
            self.comment = byte == b':';
        }
        self.line = self.line.saturating_add(1);
        if self.retained.saturating_add(self.line) > self.maximum {
            return Err(OverLimit);
        }
        Ok(())
    }
    const fn finish_line(&mut self) -> Result<(), OverLimit> {
        if self.line == 0 {
            self.retained = 0;
        }
        if self.line > 0 && !self.comment {
            self.retained = self.retained.saturating_add(self.line);
        }
        self.line = 0;
        self.comment = false;
        if self.retained > self.maximum {
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
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.failed {
            return Poll::Ready(None);
        }
        match self.inner.as_mut().poll_next(cx) {
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
