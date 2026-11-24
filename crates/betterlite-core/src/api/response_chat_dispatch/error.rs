use std::error::Error;

use thiserror::Error;

use crate::api::messages_stream_split::errors::StreamFailure;
use crate::api::response_chat_request::error::RequestError;
use crate::api::response_chat_result::error::ResponseChatResultError;
use crate::api::responses_to_chat_stream::error::ResponsesChatStreamError;
use crate::message_utilities::response_streams::StreamError;

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
    #[error("the model response could not become a stream: {source}")]
    Mock {
        #[source]
        source: StreamError,
    },
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
