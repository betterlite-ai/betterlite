use thiserror::Error;

use crate::api::messages_response_events::error::MessagesResponseEventError;
use crate::api::messages_stream_logging::ChunkError;

// The two ways __anext__ can fail outside the stream itself, and the chunk shape B1119 never queues.
#[derive(Debug, Error)]
pub enum MessagesResponseStreamError {
    #[error("the Responses event could not be translated: {0}")]
    Event(#[from] MessagesResponseEventError),
    #[error("the Anthropic chunk cannot be written as an SSE frame: {0}")]
    Sse(#[from] ChunkError),
    #[error("an Anthropic chunk is not a JSON object")]
    ChunkNotObject,
}
