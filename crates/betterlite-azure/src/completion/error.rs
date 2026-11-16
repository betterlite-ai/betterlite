use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::text_completion_transport::logging::SecretHeaders;
use betterlite_openai::text_completion_transport::sdk::SdkError;
use thiserror::Error;

use crate::endpoints::initialize::EndpointError;

pub const MISSING_MODEL_OR_MESSAGES: &str = "Missing model or messages";
pub const MAX_RETRIES_TYPE: &str = "max retries must be an int";
pub const SYNC_CLIENT: &str = "azure_client is not an instance of AzureOpenAI";
pub const ASYNC_CLIENT: &str = "azure_client is not an instance of AsyncAzureOpenAI";
pub const STREAM_ANSWER: &str = "the answer is a stream, not a completion";
pub const BUFFERED_ANSWER: &str = "the answer is not a stream";

// litellm's AzureOpenAIError: the status, text and headers the source exception had
#[derive(Debug, Error)]
#[error("{message}")]
pub struct AzureTextError {
    pub status: Option<u16>,
    pub message: String,
    pub headers: Option<SecretHeaders>,
}

impl AzureTextError {
    #[must_use]
    pub fn raised(status: u16, message: impl Into<String>) -> Self {
        Self {
            status: Some(status),
            message: message.into(),
            headers: None,
        }
    }

    #[must_use]
    pub fn internal(message: impl Into<String>) -> Self {
        Self::raised(500, message)
    }

    // getattr(error, "status_code", 500), getattr(error, "headers", None) or response.headers
    #[must_use]
    pub fn wrapped(source: &SdkError) -> Self {
        match source {
            SdkError::Status(status) => Self {
                status: status.status,
                message: status.message.clone(),
                headers: Some(SecretHeaders::from_response(&status.headers)),
            },
            SdkError::Decode(_) | SdkError::Sse(_) | SdkError::Other { .. } => {
                Self::internal(source.to_string())
            }
        }
    }

    #[must_use]
    pub fn from_endpoint(error: &EndpointError) -> Self {
        Self {
            status: Some(error.status().unwrap_or(500)),
            message: error.to_string(),
            headers: None,
        }
    }

    #[must_use]
    pub fn from_logging(error: &PrePostError) -> Self {
        Self::internal(error.to_string())
    }
}

// The sync streaming call has no try/except, so its SDK, client and logging errors reach the caller unwrapped
#[derive(Debug, Error)]
pub enum AzureTextFailure {
    #[error(transparent)]
    Azure(#[from] AzureTextError),
    #[error(transparent)]
    Raw(#[from] SdkError),
    #[error(transparent)]
    Endpoint(#[from] EndpointError),
    #[error(transparent)]
    Logging(#[from] PrePostError),
}

impl AzureTextFailure {
    #[must_use]
    pub fn into_azure(self) -> AzureTextError {
        match self {
            Self::Azure(error) => error,
            Self::Raw(source) => AzureTextError::wrapped(&source),
            Self::Endpoint(source) => AzureTextError::from_endpoint(&source),
            Self::Logging(source) => AzureTextError::from_logging(&source),
        }
    }
}
