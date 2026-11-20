use betterlite_models::decisions::Json;

use crate::api::messages_stream_logging::{SseChunk, convert_chunk_to_sse_format};

use super::error::MessagesResponseStreamError;

// async_anthropic_sse_wrapper's dict branch: the event line and the data line exactly as Python's json.dumps writes them.
pub(super) fn frame(chunk: Json) -> Result<Vec<u8>, MessagesResponseStreamError> {
    match chunk {
        Json::Object(fields) => Ok(convert_chunk_to_sse_format(SseChunk::Mapping(fields))?),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            Err(MessagesResponseStreamError::ChunkNotObject)
        }
    }
}
