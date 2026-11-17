use thiserror::Error;

use crate::api::chat_response_stream_items::error::StreamItemsError;
use crate::api::chat_response_stream_tools::error::StreamToolsError;
use crate::api::messages_stream_split::errors::StreamFailure;
use crate::api::responses_chat_output::error::ChatOutputError;

#[derive(Debug, Error)]
pub enum ChatResponseStreamError {
    #[error("the provider stream failed before its next chunk")]
    Stream { failure: StreamFailure },
    #[error("the provider chunk is not a JSON object")]
    InvalidChunk,
    #[error("the provider chunk could not be read: {source}")]
    Chunk {
        #[from]
        source: serde_json::Error,
    },
    #[error("the completed response is not a chat completion: {source}")]
    Completion { source: serde_json::Error },
    #[error("the Responses request could not be read: {source}")]
    Request { source: serde_json::Error },
    #[error("the {field} metadata is not a string")]
    Metadata { field: &'static str },
    #[error("the {field} delta is not a string")]
    FieldNotText { field: &'static str },
    #[error(transparent)]
    Items(#[from] StreamItemsError),
    #[error(transparent)]
    Tools(#[from] StreamToolsError),
    #[error(transparent)]
    Output(#[from] ChatOutputError),
}
