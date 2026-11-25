use std::error::Error;

use betterlite_streaming::provider_chunk_decode::wrapper::WrapperError;
use thiserror::Error;

use crate::api::chat_response_stream::error::ChatResponseStreamError;
use crate::api::messages_stream_split::errors::StreamFailure;
use crate::api::response_chat_request::error::RequestError;
use crate::api::response_chat_result::error::ResponseChatResultError;
use crate::api::responses_to_chat_stream::error::ResponsesChatStreamError;

#[derive(Debug, Error)]
pub enum InputError {
    #[error("{field} is required")]
    Required { field: &'static str },
    #[error("{field} must be {expected}")]
    Field {
        field: &'static str,
        expected: &'static str,
    },
    #[error(transparent)]
    Request(#[from] RequestError),
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct PostStreamError {
    pub message: String,
}

// One stream step's own failure, kept typed with its source until the AsyncChunkStream boundary, which carries text
#[derive(Debug, Error)]
pub enum StreamStepError {
    #[error(transparent)]
    Translate(#[from] ResponsesChatStreamError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Wrapper(#[from] WrapperError),
    #[error(transparent)]
    View(#[from] ChatResponseStreamError),
}

impl From<StreamStepError> for StreamFailure {
    fn from(error: StreamStepError) -> Self {
        Self::Unclassified {
            text: error.to_string(),
        }
    }
}

// Every way the bridge stops short of an answer, with the provider's own failure kept as the cause
#[derive(Debug, Error)]
pub enum DispatchError<Source>
where
    Source: Error + 'static,
{
    #[error(transparent)]
    Input(#[from] InputError),
    #[error(transparent)]
    Request(#[from] RequestError),
    #[error(transparent)]
    Transform(#[from] ResponseChatResultError),
    #[error(transparent)]
    Translate(#[from] ResponsesChatStreamError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("the completed responses payload could not be read: {source}")]
    Completion {
        #[source]
        source: serde_path_to_error::Error<serde_json::Error>,
    },
    #[error("Unexpected responses stream payload")]
    UnexpectedPayload,
    #[error("Stream ended without a completed response")]
    MissingCompletedResponse,
    #[error("the responses stream failed")]
    Stream { failure: StreamFailure },
    #[error(transparent)]
    PostStream(#[from] PostStreamError),
    #[error(transparent)]
    Provider(Source),
}
