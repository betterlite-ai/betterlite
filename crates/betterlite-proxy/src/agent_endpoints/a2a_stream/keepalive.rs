use std::time::Duration;

use futures_util::StreamExt as _;
use futures_util::stream::unfold;
use tokio::time::timeout;

use super::FrameStream;

// an SSE comment, which every conformant client discards
pub const PING: &str = ": ping\n\n";

#[must_use]
pub fn coerce_keepalive_interval(seconds: Option<f64>) -> Option<Duration> {
    let interval = seconds?;
    if !interval.is_finite() || interval <= 0.0_f64 {
        return None;
    }
    Duration::try_from_secs_f64(interval).ok()
}

// StreamExt::next is cancel safe, so a ping leaves the inner stream where it was
#[must_use]
pub fn with_pings(frames: FrameStream, interval: Duration) -> FrameStream {
    unfold(frames, move |mut inner| async move {
        let frame = match timeout(interval, inner.next()).await {
            Err(_idle) => Ok(String::from(PING)),
            Ok(next) => next?,
        };
        Some((frame, inner))
    })
    .boxed()
}

mod tests;
