use std::num::TryFromIntError;
use std::str::Utf8Error;

use thiserror::Error;

use crate::api::api_boundaries::ApiLimitError;
use crate::api::response_chat_history::error::HistoryError;
use crate::api::responses_usage::ResponsesUsageError;

#[derive(Debug, Error)]
pub enum ResponsesChatStreamError {
    #[error(transparent)]
    Limit(#[from] ApiLimitError),
    #[error("the provider event line is not UTF-8")]
    Utf8(#[from] Utf8Error),
    #[error("{message}")]
    Json { message: String },
    #[error("Chat provider: Empty parsed_chunk")]
    EmptyChunk,
    #[error("Chat provider: Invalid chunk type <class '{kind}'>")]
    InvalidChunkType { kind: &'static str },
    #[error("Chat provider: Invalid function argument delta {event}")]
    InvalidArgumentDelta { event: String },
    #[error("Chat provider: Invalid text delta {event}")]
    InvalidTextDelta { event: String },
    #[error("the provider event field `{field}` has the wrong type")]
    Field { field: &'static str },
    #[error("the tool call count does not fit in a chat index")]
    IndexCount {
        #[source]
        source: TryFromIntError,
    },
    #[error(transparent)]
    History(#[from] HistoryError),
    #[error(transparent)]
    Usage(#[from] ResponsesUsageError),
}
