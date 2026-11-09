use std::str::Utf8Error;

use betterlite_context::a2a_text::A2aTextError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentCoreReplyError {
    #[error("the response body is not UTF-8")]
    NotUtf8 {
        #[source]
        source: Utf8Error,
    },
    #[error("the response body is not valid JSON")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the A2A response could not be decoded")]
    A2aShape {
        #[source]
        source: serde_json::Error,
    },
    #[error("the A2A response could not be re-encoded")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the A2A response text could not be extracted")]
    A2aText {
        #[source]
        source: A2aTextError,
    },
    #[error("event contentBlockDelta must be an object")]
    DeltaEventNotObject,
    #[error("event contentBlockDelta.delta must be an object")]
    DeltaNotObject,
    #[error("event contentBlockDelta.delta.text must be a string")]
    DeltaTextNotString,
    #[error("event metadata must be an object")]
    MetadataNotObject,
    #[error("event metadata usage must be an object")]
    UsageNotObject,
    #[error("event metadata usage field {field} must be an integer")]
    UsageFieldNotInteger { field: &'static str },
    #[error("event metadata usage field {field} cannot be negative")]
    UsageFieldNegative { field: &'static str },
    #[error("message content block {index} text must be a string")]
    ContentTextNotString { index: usize },
}
