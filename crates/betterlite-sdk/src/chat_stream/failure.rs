use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;

use super::ChatStream;
use super::config::CallMode;
use super::error::{self, ChatStreamCause, ChatStreamError};
use super::finalize;
use super::ports::{ChunkSource, SourceFailure, StreamFailure, StreamSourceError};

// _log_stream_failure_and_raise, then _handle_stream_fallback_error.
pub(super) fn failed<S>(stream: &ChatStream<'_, S>, cause: ChatStreamCause) -> ChatStreamError {
    let class = error::mapped(&cause, &stream.config, stream.ports.redactor);
    let failure = StreamFailure {
        class: class.clone(),
        message: cause.to_string(),
    };
    if matches!(stream.config.mode, CallMode::Async) && !stream.shaper.chunks.is_empty() {
        stream.ports.logging.partial_usage(&stream.shaper.chunks);
    }
    stream.ports.logging.log_failure(&failure);
    error::wrap(class, cause, stream)
}

// A read or protocol error after a finish reason ends the async stream; any other failure raises.
pub(super) async fn source_failed<S>(
    stream: &mut ChatStream<'_, S>,
    error: StreamSourceError,
) -> Result<Option<StreamChunk>, ChatStreamError>
where
    S: ChunkSource,
{
    let ends_stream = matches!(stream.config.mode, CallMode::Async)
        && matches!(error.failure, SourceFailure::Read | SourceFailure::Protocol)
        && stream.shaper.received_finish_reason.is_some();
    if !ends_stream {
        return Err(failed(stream, ChatStreamCause::Source(error)));
    }
    match finalize::end_of_source(stream).await {
        Ok(chunk) => Ok(chunk),
        Err(cause) => Err(error::direct(cause, stream)),
    }
}
