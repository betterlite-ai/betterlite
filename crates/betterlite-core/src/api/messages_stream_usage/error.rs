use thiserror::Error;

use crate::api::messages_chat_output::error::ChatOutputError;

// Each state LiteLLM raises from sits on a path its own iteration avoids, so a caller that reaches one gets an error
#[derive(Debug, Error)]
pub enum StreamStateError {
    #[error("the held stop-reason chunk is missing")]
    MissingHeldStop,
    #[error("the event cannot be read: the {field} field is missing or of the wrong type")]
    MalformedEvent { field: &'static str },
    #[error("the stream state cannot read the chunk: {source}")]
    Content {
        #[from]
        source: ChatOutputError,
    },
    #[error("the event is not valid JSON: {source}")]
    Serialization {
        #[from]
        source: serde_json::Error,
    },
}
