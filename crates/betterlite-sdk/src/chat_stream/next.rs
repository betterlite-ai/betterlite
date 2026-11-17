use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;

use super::ChatStream;
use super::chunk::{self, Step};
use super::config::CallMode;
use super::error::{self, ChatStreamError};
use super::ports::ChunkSource;
use super::{duration, failure, finalize};

// __anext__ and, for CallMode::Blocking, __next__: the duration check, then chunks until the source ends.
pub(super) async fn advance<S>(
    stream: &mut ChatStream<'_, S>,
) -> Result<Option<StreamChunk>, ChatStreamError>
where
    S: ChunkSource,
{
    if let Some(cause) = duration::exceeded(stream) {
        return match stream.config.mode {
            CallMode::Blocking => Err(error::direct(cause, stream)),
            CallMode::Async => Err(failure::failed(stream, cause)),
        };
    }
    loop {
        let raw = match stream.source.next_chunk().await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => return end_of_stream(stream).await,
            Err(error) => return failure::source_failed(stream, error).await,
        };
        if chunk::skip_source_chunk(stream.config.mode, &raw) {
            continue;
        }
        match chunk::process(stream, raw).await {
            Ok(Step::Chunk(chunk)) => return Ok(Some(*chunk)),
            Ok(Step::Skip) => {}
            Ok(Step::End) => return end_of_stream(stream).await,
            Err(cause) => return Err(failure::failed(stream, cause)),
        }
    }
}

// The source and its finalize failures are reported as mapped, without the fallback wrapper.
async fn end_of_stream<S>(
    stream: &mut ChatStream<'_, S>,
) -> Result<Option<StreamChunk>, ChatStreamError>
where
    S: ChunkSource,
{
    match finalize::end_of_source(stream).await {
        Ok(chunk) => Ok(chunk),
        Err(cause) => Err(error::direct(cause, stream)),
    }
}
