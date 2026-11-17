use thiserror::Error;

use crate::api::messages_stream_logging::ChunkError;
use crate::api::messages_stream_split::errors::StreamFailure;
use crate::api::messages_stream_usage::error::StreamStateError;

// The async iteration lets every failure reach its caller; only the SSE boundary turns one into an error event
#[derive(Debug, Error)]
pub enum ChatStreamError {
    #[error("the provider stream failed before the wrapper could translate a chunk")]
    Stream { failure: StreamFailure },
    #[error("the stream state rejected a chunk: {0}")]
    State(#[from] StreamStateError),
}

impl ChatStreamError {
    // B975 reads a status off a reported failure and gives every other one the generic 500 wording
    #[must_use]
    pub fn into_stream_failure(self) -> StreamFailure {
        match self {
            Self::Stream { failure } => failure,
            Self::State(error) => StreamFailure::Unclassified {
                text: error.to_string(),
            },
        }
    }
}

#[derive(Debug, Error)]
pub enum ChatStreamEncodeError {
    #[error("the event is not a JSON object: {0}")]
    Json(#[from] serde_json::Error),
    #[error("the event cannot name an SSE event: {0}")]
    Chunk(#[from] ChunkError),
}
