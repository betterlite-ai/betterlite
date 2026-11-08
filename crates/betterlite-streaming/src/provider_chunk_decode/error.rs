use std::str::Utf8Error;

use thiserror::Error;

use crate::stream_chunk_shaping::StreamShapeError;

#[derive(Debug, Error)]
pub enum ProviderChunkError {
    #[error("the provider stream is finished")]
    StopIteration,
    #[error("Unable to parse response. Original response: {chunk}")]
    ParseResponse {
        chunk: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("Unable to parse response. Original response: {chunk}")]
    RejectedResponse { chunk: String },
    #[error("{message}")]
    ProviderReported { message: String },
    #[error("An unknown error occurred with the stream")]
    UnknownStream,
    #[error("The response was blocked by VertexAI.")]
    VertexBlocked,
    #[error("chunk is not a string: {chunk}")]
    NotAString { chunk: String },
    #[error("the provider stream did not send {expected}")]
    UnexpectedChunk { expected: &'static str },
    #[error("the provider chunk has no {field} field")]
    MissingField { field: &'static str },
    #[error("the provider chunk field {field} has the wrong type")]
    InvalidField { field: &'static str },
    #[error("the provider chunk is not valid UTF-8")]
    InvalidUtf8 {
        #[from]
        source: Utf8Error,
    },
    #[error("{provider} raised a streaming error - finish_reason: error, no content string given")]
    StreamingFailure { provider: String },
    #[error("the provider chunk does not match the OpenAI chunk shape: {source}")]
    InvalidChunk {
        #[from]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Shape {
        #[from]
        source: StreamShapeError,
    },
}

impl ProviderChunkError {
    // The Python class name litellm's exception_type maps from, for the driver.
    #[must_use]
    pub const fn class(&self) -> &'static str {
        match self {
            Self::StopIteration => "StopIteration",
            Self::ParseResponse { .. }
            | Self::RejectedResponse { .. }
            | Self::NotAString { .. }
            | Self::Shape { .. } => "ValueError",
            Self::ProviderReported { .. }
            | Self::UnknownStream
            | Self::VertexBlocked
            | Self::StreamingFailure { .. } => "Exception",
            Self::UnexpectedChunk { .. }
            | Self::MissingField { .. }
            | Self::InvalidField { .. }
            | Self::InvalidChunk { .. } => "TypeError",
            Self::InvalidUtf8 { .. } => "UnicodeDecodeError",
        }
    }
}
