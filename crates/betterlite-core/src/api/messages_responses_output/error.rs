use thiserror::Error;

use crate::api::api_boundaries::ApiLimitError;
use crate::api::messages_responses_input::error::MessagesResponsesInputError;
use crate::api::messages_thinking::session::SessionKeyError;

#[derive(Debug, Error)]
pub enum MessagesResponsesOutputError {
    #[error(transparent)]
    Limit(#[from] ApiLimitError),
    #[error("a Responses function call carries arguments that are not JSON: {source}")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("a Responses function call carries arguments that are not a JSON object")]
    ArgumentsNotObject,
    #[error("the Responses output cannot be read as a JSON document: {source}")]
    Output {
        #[source]
        source: serde_json::Error,
    },
    #[error("the request messages do not translate to Responses input: {source}")]
    Messages {
        #[source]
        source: MessagesResponsesInputError,
    },
    #[error("the Responses usage does not translate to Anthropic usage: {source}")]
    Usage {
        #[source]
        source: MessagesResponsesInputError,
    },
    #[error("metadata.user_id cannot be read as a session key: {source}")]
    SessionKey {
        #[source]
        source: SessionKeyError,
    },
    #[error("context_management.edits[].trigger.value is not an integer")]
    ContextThreshold,
    #[error("output_config.effort {effort:?} is not a reasoning effort")]
    Effort { effort: String },
}
