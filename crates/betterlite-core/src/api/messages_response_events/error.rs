use betterlite_anthropic::api::messages_errors::decode::WireDecodeError;

use crate::api::messages_responses_input::error::MessagesResponsesInputError;

#[derive(Debug, thiserror::Error)]
pub enum MessagesResponseEventError {
    #[error("a response.failed event carries a response that is not a mapping or an object")]
    FailedEvent,
    #[error("the Responses usage could not become Anthropic usage: {0}")]
    Usage(#[source] MessagesResponsesInputError),
    #[error("the Anthropic error envelope could not be mapped: {0}")]
    ErrorEnvelope(#[source] WireDecodeError),
    #[error("a chunk or the error envelope is not JSON: {0}")]
    Json(#[source] serde_json::Error),
}
