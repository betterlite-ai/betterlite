use std::str::Utf8Error;

use betterlite_tokenizer::a2a_text::A2aTextError;
use thiserror::Error;

use crate::bedrock_settings::{BedrockError, HeaderError};

// B682's BedrockError implements no Display or Error, so the derived wrapper carries the cause instead.
#[derive(Debug, Error)]
#[error("bedrock provider error")]
pub struct BedrockFailure {
    pub cause: Box<BedrockError>,
}

#[derive(Debug, Error)]
pub enum AgentCoreResponseError {
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
    #[error("bedrock provider error ({status})")]
    Bedrock {
        status: u16,
        #[source]
        error: BedrockFailure,
    },
    #[error(transparent)]
    Header(#[from] HeaderError),
}
