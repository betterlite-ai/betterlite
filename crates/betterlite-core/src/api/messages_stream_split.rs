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

impl<S: ChunkSource> Iterator for ChunkSplitter<S> {
    type Item = Result<StreamChunk, errors::StreamFailure>;

    fn next(&mut self) -> Option<Self::Item> {
        next_piece(&mut self.source, &mut self.buffer).transpose()
    }
}

pub struct AsyncChunkSplitter<S> {
    pub source: S,
    pub buffer: VecDeque<StreamChunk>,
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

pub async fn next_split_chunk<S>(
    splitter: &mut AsyncChunkSplitter<S>,
) -> Result<Option<StreamChunk>, errors::StreamFailure>
where
    S: AsyncChunkSource + Send,
{
    if let Some(piece) = splitter.buffer.pop_front() {
        return Ok(Some(piece));
    }
    let Some(chunk) = splitter.source.next_chunk().await? else {
        return Ok(None);
    };
    splitter.buffer.extend(split::split_chunk(&chunk));
    Ok(splitter.buffer.pop_front())
}
