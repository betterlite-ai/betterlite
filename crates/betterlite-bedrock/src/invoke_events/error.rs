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
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::Frame(EventFrameError::Bedrock { status, .. })
            | Self::OpenAi(StreamError::Provider { status, .. }) => Some(*status),
            // LiteLLM raises AnthropicError(status_code=500) for a native stream error event.
            Self::Anthropic(StreamEventError::Anthropic(error)) => Some(error.status_code),
            Self::Frame(
                EventFrameError::PreludeChecksum { .. }
                | EventFrameError::Frame(_)
                | EventFrameError::DuplicateHeader { .. }
                | EventFrameError::NotUtf8(_)
                | EventFrameError::NotJson(_)
                | EventFrameError::PayloadNotObject
                | EventFrameError::ChunkBytesNotText
                | EventFrameError::ChunkBase64(_)
                | EventFrameError::Header(_),
            )
            | Self::OpenAi(StreamError::Chunk(_) | StreamError::ToolCallPayload)
            | Self::Anthropic(StreamEventError::Shape(_) | StreamEventError::State { .. })
            | Self::Json { .. }
            | Self::Chunk { .. }
            | Self::Encode { .. }
            | Self::Converse(_)
            | Self::Buffered { .. } => None,
        }
    }
}
