use async_trait::async_trait;
use betterlite_models::decisions::Json;

use crate::api::messages_stream_split::errors::StreamFailure;

// One chunk as ModelResponseStream dumps it, built with chunk::wrapper_view, or the failure the wrapper raised
pub trait ChunkStream {
    fn next_chunk(&mut self) -> Result<Option<Json>, StreamFailure>;
}

#[async_trait]
pub trait AsyncChunkStream {
    async fn next_chunk(&mut self) -> Result<Option<Json>, StreamFailure>;
}
