use std::convert::Infallible;

use betterlite_anthropic::stream_events::error::StreamEventError;
use betterlite_openai::chat_choices::stream::StreamError;
use thiserror::Error;

use crate::converse_events::error::ConverseEventError;
use crate::event_frames::error::EventFrameError;

#[derive(Debug, Error)]
pub enum InvokeStreamError {
    #[error("the Bedrock event payload is not valid JSON")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Frame(#[from] EventFrameError),
    #[error("the provider stream chunk is invalid: {message}")]
    Chunk { message: String },
    #[error("the provider chunk could not be encoded for projection")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the Anthropic stream event could not be projected")]
    Anthropic(#[source] StreamEventError<Infallible>),
    #[error("the Converse stream event could not be projected")]
    Converse(#[source] ConverseEventError),
    #[error("the OpenAI-compatible stream chunk could not be projected")]
    OpenAi(#[source] StreamError),
    #[error("Failed to decode chunk: {chunk}. Error: {error}")]
    Buffered { chunk: String, error: String },
}

impl InvokeStreamError {
    #[must_use]
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Frame(EventFrameError::Bedrock { status, .. }) => Some(*status),
            Self::OpenAi(StreamError::Provider { status, .. }) => Some(*status),
            Self::Json { .. }
            | Self::Chunk { .. }
            | Self::Encode { .. }
            | Self::Anthropic(_)
            | Self::Converse(_)
            | Self::Buffered { .. } => None,
        }
    }
}
