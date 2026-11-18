use std::pin::Pin;
use std::task::{Context, Poll};

use futures_util::Stream;
use http::StatusCode;

use crate::logging_service_events::pipeline::ServiceCall;

pub const STREAM_FAILED_ERROR: &str = "the response stream failed";

// A body that records the call's one terminal event: at stream end, on a stream error, or when dropped.
pub struct ServiceEventStream<S> {
    inner: Pin<Box<S>>,
    call: Option<ServiceCall>,
    failure: Option<String>,
}

#[must_use]
pub fn failure_for(status: StatusCode) -> Option<String> {
    (!status.is_success())
        .then(|| format!("the upstream call failed with HTTP {}", status.as_u16()))
}

impl<S> ServiceEventStream<S> {
    #[must_use]
    pub fn new(inner: S, call: ServiceCall, failure: Option<String>) -> Self {
        Self {
            inner: Box::pin(inner),
            call: Some(call),
            failure,
        }
    }
}

impl<S, Data, Error> Stream for ServiceEventStream<S>
where
    S: Stream<Item = Result<Data, Error>>,
{
    type Item = Result<Data, Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        let polled = this.inner.as_mut().poll_next(cx);
        let failure = match &polled {
            Poll::Ready(None) => this.failure.take(),
            Poll::Ready(Some(Err(_error))) => Some(STREAM_FAILED_ERROR.to_owned()),
            Poll::Ready(Some(Ok(_))) | Poll::Pending => return polled,
        };
        if let Some(call) = this.call.take() {
            call.finish(failure);
        }
        polled
    }
}

// A dropped stream leaves its call unsettled, and ServiceCall's own Drop records the cancellation.
