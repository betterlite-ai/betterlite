use std::pin::Pin;
use std::task::{Context, Poll};

use futures_util::Stream;

// the guard releases a slot when the response body finishes or the client drops it
pub(super) struct GuardedBody<Source, Guard> {
    pub(super) inner: Source,
    pub(super) guard: Option<Guard>,
}

impl<Source, Guard> Stream for GuardedBody<Source, Guard>
where
    Source: Stream + Unpin,
    Guard: Unpin,
{
    type Item = Source::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        let polled = Pin::new(&mut this.inner).poll_next(cx);
        if matches!(polled, Poll::Ready(None)) {
            this.guard.take();
        }
        polled
    }
}
