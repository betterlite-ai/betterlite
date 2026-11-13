use async_trait::async_trait;
use betterlite_core::api::messages_stream_split::errors::StreamFailure;
use betterlite_core::api::messages_stream_split::{AsyncChunkSource, ChunkSource};
use betterlite_models::decisions::Json;
use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;

#[derive(Clone)]
pub(super) struct ScriptedSource {
    pub(super) chunks: Vec<StreamChunk>,
    pub(super) next: usize,
    pub(super) failure: Option<StreamFailure>,
    pub(super) chunks_attr: Option<Vec<StreamChunk>>,
    pub(super) messages_attr: Option<Vec<Json>>,
    pub(super) pulls: usize,
}

impl ScriptedSource {
    fn pull(&mut self) -> Result<Option<StreamChunk>, StreamFailure> {
        self.pulls = self.pulls.saturating_add(1);
        if let Some(chunk) = self.chunks.get(self.next) {
            self.next = self.next.saturating_add(1);
            return Ok(Some(chunk.clone()));
        }
        self.failure.clone().map_or(Ok(None), Err)
    }
}

impl ChunkSource for ScriptedSource {
    fn next_chunk(&mut self) -> Result<Option<StreamChunk>, StreamFailure> {
        self.pull()
    }

    fn chunks(&self) -> Option<&[StreamChunk]> {
        self.chunks_attr.as_deref()
    }

    fn messages(&self) -> Option<&[Json]> {
        self.messages_attr.as_deref()
    }
}

#[async_trait]
impl AsyncChunkSource for ScriptedSource {
    async fn next_chunk(&mut self) -> Result<Option<StreamChunk>, StreamFailure> {
        self.pull()
    }

    fn chunks(&self) -> Option<&[StreamChunk]> {
        self.chunks_attr.as_deref()
    }

    fn messages(&self) -> Option<&[Json]> {
        self.messages_attr.as_deref()
    }
}
