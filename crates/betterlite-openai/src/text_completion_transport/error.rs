use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_streaming::provider_chunk_decode::error::ProviderChunkError;
use thiserror::Error;

use super::logging::SecretHeaders;
use super::sdk::SdkError;
use crate::text_completion_policy::prompt::PromptError;

// status_code as the caught exception carries it: absent is 500, an explicit None stays None
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorStatus {
    Missing,
    Null,
    Code(u16),
}

#[derive(Debug, Error)]
pub enum ErrorSource {
    #[error(transparent)]
    Sdk(Box<SdkError>),
    #[error(transparent)]
    Transform(#[from] PromptError),
    #[error(transparent)]
    Logging(#[from] PrePostError),
    #[error(transparent)]
    Chunk(#[from] ProviderChunkError),
    #[error("Missing model or messages")]
    Raised,
    #[error(transparent)]
    Serialize(serde_json::Error),
    #[error("{message}")]
    Invalid { message: String },
}

// litellm's OpenAIError from common_utils: the status, text and headers the source exception had
#[derive(Debug, Error)]
#[error("{message}")]
pub struct TextCompletionError {
    pub status: ErrorStatus,
    pub message: String,
    pub headers: Option<SecretHeaders>,
    #[source]
    pub source: Box<ErrorSource>,
}

impl TextCompletionError {
    pub(super) fn wrapped(source: ErrorSource) -> Self {
        let (status, headers) = match &source {
            ErrorSource::Sdk(error) => match error.as_ref() {
                SdkError::Status(status) => (
                    status.status.map_or(ErrorStatus::Null, ErrorStatus::Code),
                    Some(SecretHeaders::from_response(&status.headers)),
                ),
                SdkError::Decode(_) | SdkError::Sse(_) | SdkError::Other { .. } => {
                    (ErrorStatus::Missing, None)
                }
            },
            ErrorSource::Raised => (ErrorStatus::Code(422), None),
            ErrorSource::Transform(_)
            | ErrorSource::Logging(_)
            | ErrorSource::Chunk(_)
            | ErrorSource::Serialize(_)
            | ErrorSource::Invalid { .. } => (ErrorStatus::Missing, None),
        };
        Self {
            status,
            message: source.to_string(),
            headers,
            source: Box::new(source),
        }
    }
}

// The async paths and the sync stream's send let the original SDK exception through unwrapped
#[derive(Debug, Error)]
pub enum TextCompletionFailure {
    #[error(transparent)]
    Wrapped(#[from] TextCompletionError),
    #[error(transparent)]
    Raw(#[from] SdkError),
}

// Whether the handler wraps an SDK error in OpenAIError or lets it through unchanged
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorMode {
    Mapped,
    Raw,
}

impl TextCompletionFailure {
    pub(super) fn invalid(message: String) -> Self {
        Self::Wrapped(TextCompletionError::wrapped(ErrorSource::Invalid {
            message,
        }))
    }
}

impl From<serde_json::Error> for TextCompletionFailure {
    fn from(source: serde_json::Error) -> Self {
        Self::Wrapped(TextCompletionError::wrapped(ErrorSource::Serialize(source)))
    }
}
