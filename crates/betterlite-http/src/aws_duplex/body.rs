use std::error::Error as StdError;
use std::pin::Pin;
use std::sync::{Mutex, PoisonError};
use std::task::{Context, Poll};

use aws_smithy_types::body::SdkBody;
use bytes::Bytes;
use futures_util::{Stream, StreamExt as _};
use http_body::{Body as HttpBody, Frame, SizeHint};
use tokio_util::task::task_tracker::TaskTrackerToken;

pub(super) type BodyError = Box<dyn StdError + Send + Sync>;
pub(super) type ByteStream = Pin<Box<dyn Stream<Item = Result<Bytes, reqwest::Error>> + Send>>;

// The drain token lives in the body, so the client is drained only once every direction released it
pub(super) struct RequestBody {
    body: SdkBody,
    _io: TaskTrackerToken,
}

impl RequestBody {
    pub(super) const fn new(body: SdkBody, io: TaskTrackerToken) -> Self {
        Self { body, _io: io }
    }
}

impl HttpBody for RequestBody {
    type Data = Bytes;
    type Error = BodyError;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        Pin::new(&mut self.get_mut().body).poll_frame(cx)
    }

    fn is_end_stream(&self) -> bool {
        self.body.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.body.size_hint()
    }
}

pub(super) struct ResponseBody {
    stream: Mutex<ByteStream>,
    _io: TaskTrackerToken,
}

impl ResponseBody {
    pub(super) const fn new(stream: ByteStream, io: TaskTrackerToken) -> Self {
        Self {
            stream: Mutex::new(stream),
            _io: io,
        }
    }
}

impl HttpBody for ResponseBody {
    type Data = Bytes;
    type Error = BodyError;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        let mut stream = self
            .get_mut()
            .stream
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        match stream.as_mut().poll_next(cx) {
            Poll::Ready(Some(Ok(bytes))) => Poll::Ready(Some(Ok(Frame::data(bytes)))),
            Poll::Ready(Some(Err(source))) => Poll::Ready(Some(Err(Box::new(source)))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}
