use betterlite_error::proxy_exception::ProxyException;

use super::call::StreamAbort;
use crate::stream_chunks::StreamItem;

// LiteLLM's serializer pair: serialize_chunk, flush_tail and serialize_error
pub trait ChunkFraming: Send + 'static {
    type Out: Send + 'static;

    fn frame_chunk(&mut self, item: StreamItem) -> Result<Self::Out, StreamAbort>;

    fn frame_tail(&mut self) -> Option<Self::Out>;

    fn frame_failure(&mut self, error: &ProxyException) -> Result<Self::Out, StreamAbort>;
}
