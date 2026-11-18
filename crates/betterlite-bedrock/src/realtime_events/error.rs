use std::str::Utf8Error;

use thiserror::Error;

use crate::realtime_completion::error::CompletionError;

// LiteLLM lets the provider payload decide its own error class, so a shape that does not match keeps the source
#[derive(Debug, Error)]
pub enum EventError {
    #[error("the provider event bytes are not UTF-8")]
    Utf8 {
        #[source]
        source: Utf8Error,
    },
    #[error("the provider event is not a JSON object")]
    NotAnObject {
        #[source]
        source: serde_json::Error,
    },
    #[error("the provider event `event` is not a JSON object")]
    EventNotAnObject {
        #[source]
        source: serde_json::Error,
    },
    #[error("the provider event field `{field}` does not have the shape this event needs")]
    Payload {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("a transformed realtime event does not encode as JSON")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the {subject} exceeds the {limit} limit")]
    ExceedsLimit { subject: &'static str, limit: usize },
    #[error(transparent)]
    Completion(#[from] CompletionError),
}
