use std::time::Duration;

use bytes::Bytes;
use futures_util::Stream;
use futures_util::StreamExt as _;
use futures_util::stream::BoxStream;

use super::call::StreamAbort;
use super::sse::Emitted;
use crate::keepalive_interval::{PingSetting, coerce_keepalive_interval};

// LiteLLM's wrap_sse_stream_with_keepalive_pings: with no usable interval the stream comes back as it is
#[must_use]
pub fn with_sse_pings(
    stream: BoxStream<'static, Result<Emitted, StreamAbort>>,
    setting: Option<&PingSetting>,
    ping: &'static str,
) -> BoxStream<'static, Result<Emitted, StreamAbort>> {
    let Some(interval) = coerce_keepalive_interval(setting) else {
        return stream;
    };
    paced(stream, interval, ping).boxed()
}

// StreamExt::next is cancel safe, so a ping leaves the inner stream where it was
fn paced(
    stream: BoxStream<'static, Result<Emitted, StreamAbort>>,
    interval: Duration,
    ping: &'static str,
) -> impl Stream<Item = Result<Emitted, StreamAbort>> + Send + 'static {
    async_stream::stream! {
        let mut inner = stream;
        loop {
            match tokio::time::timeout(interval, inner.next()).await {
                Err(_elapsed) => yield Ok(Emitted::Frame(Bytes::from_static(ping.as_bytes()))),
                Ok(None) => return,
                Ok(Some(item)) => yield item,
            }
        }
    }
}
