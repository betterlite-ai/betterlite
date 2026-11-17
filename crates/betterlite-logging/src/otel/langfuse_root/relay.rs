use std::pin::Pin;
use std::task::{Context, Poll};

use futures_util::Stream;

use crate::otel::content_projections::{RelayedChunk, stream_output};
use crate::otel::langfuse_root::LangfuseContentOtel;
use crate::otel::model::metadata::wire::MetadataMap;

// litellm.integrations.otel.langfuse_logger's streaming hook: yield every chunk as it arrives and stamp the root only at the tail
pub struct StampingStream<'stream, S> {
    pub(crate) inner: Pin<Box<S>>,
    pub(crate) logger: &'stream LangfuseContentOtel,
    pub(crate) request_data: &'stream MetadataMap,
    pub(crate) relayed: Vec<RelayedChunk>,
    pub(crate) finished: bool,
}

impl<S> Stream for StampingStream<'_, S>
where
    S: Stream<Item = RelayedChunk>,
{
    type Item = RelayedChunk;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.finished {
            return Poll::Ready(None);
        }
        match this.inner.as_mut().poll_next(cx) {
            Poll::Ready(Some(chunk)) => {
                this.relayed.push(chunk.clone());
                Poll::Ready(Some(chunk))
            }
            Poll::Ready(None) => {
                this.finished = true;
                let logger = this.logger;
                let data = this.request_data;
                logger.stamp_root_io(data, || {
                    stream_output(&this.relayed, data, logger.stream_assembler.as_ref())
                });
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}
