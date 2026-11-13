use std::pin::Pin;
use std::task::{Context, Poll};

use http_body::{Body, Frame, SizeHint};

// the guard releases a slot when the response body finishes or the client drops it
pub(super) struct GuardedBody<Inner, Guard> {
    pub(super) inner: Inner,
    pub(super) guard: Option<Guard>,
}

impl<Inner, Guard> Body for GuardedBody<Inner, Guard>
where
    Inner: Body + Unpin,
    Guard: Unpin,
{
    type Data = Inner::Data;
    type Error = Inner::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();
        let polled = Pin::new(&mut this.inner).poll_frame(cx);
        if matches!(polled, Poll::Ready(None)) {
            this.guard.take();
        }
        polled
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}
