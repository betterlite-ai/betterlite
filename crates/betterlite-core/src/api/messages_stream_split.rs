use std::collections::VecDeque;

use async_trait::async_trait;
use betterlite_models::decisions::Json;
use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;

pub mod delta;
pub mod errors;
mod split;
mod tests;

pub trait ChunkSource {
    fn next_chunk(&mut self) -> Result<Option<StreamChunk>, errors::StreamFailure>;

    // LiteLLM forwards the inner stream's request chunks and messages for logging, and null when it has none
    fn chunks(&self) -> Option<&[StreamChunk]> {
        None
    }

    fn messages(&self) -> Option<&[Json]> {
        None
    }
}

#[async_trait]
pub trait AsyncChunkSource {
    async fn next_chunk(&mut self) -> Result<Option<StreamChunk>, errors::StreamFailure>;

    fn chunks(&self) -> Option<&[StreamChunk]> {
        None
    }

    fn messages(&self) -> Option<&[Json]> {
        None
    }
}

// An instance pulls from one source in one iteration mode only; its buffered pieces drain before the next pull
pub struct ChunkSplitter<S> {
    pub source: S,
    pub buffer: VecDeque<StreamChunk>,
}

impl<S: ChunkSource> ChunkSource for ChunkSplitter<S> {
    fn next_chunk(&mut self) -> Result<Option<StreamChunk>, errors::StreamFailure> {
        next_piece(&mut self.source, &mut self.buffer)
    }

    fn chunks(&self) -> Option<&[StreamChunk]> {
        self.source.chunks()
    }

    fn messages(&self) -> Option<&[Json]> {
        self.source.messages()
    }
}

impl<S: ChunkSource> Iterator for ChunkSplitter<S> {
    type Item = Result<StreamChunk, errors::StreamFailure>;

    fn next(&mut self) -> Option<Self::Item> {
        <Self as ChunkSource>::next_chunk(self).transpose()
    }
}

pub struct AsyncChunkSplitter<S> {
    pub source: S,
    pub buffer: VecDeque<StreamChunk>,
}

#[async_trait]
impl<S> AsyncChunkSource for AsyncChunkSplitter<S>
where
    S: AsyncChunkSource + Send,
{
    async fn next_chunk(&mut self) -> Result<Option<StreamChunk>, errors::StreamFailure> {
        if let Some(piece) = self.buffer.pop_front() {
            return Ok(Some(piece));
        }
        let Some(chunk) = self.source.next_chunk().await? else {
            return Ok(None);
        };
        self.buffer.extend(split::split_chunk(&chunk));
        Ok(self.buffer.pop_front())
    }

    fn chunks(&self) -> Option<&[StreamChunk]> {
        self.source.chunks()
    }

    fn messages(&self) -> Option<&[Json]> {
        self.source.messages()
    }
}

fn next_piece<S>(
    source: &mut S,
    buffer: &mut VecDeque<StreamChunk>,
) -> Result<Option<StreamChunk>, errors::StreamFailure>
where
    S: ChunkSource,
{
    if let Some(piece) = buffer.pop_front() {
        return Ok(Some(piece));
    }
    let Some(chunk) = source.next_chunk()? else {
        return Ok(None);
    };
    buffer.extend(split::split_chunk(&chunk));
    Ok(buffer.pop_front())
}
