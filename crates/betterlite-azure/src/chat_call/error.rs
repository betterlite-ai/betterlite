use betterlite_models::decisions::Json;
use betterlite_openai::output_limits::raw_send::RawChatError;
use betterlite_openai::text_completion_transport::logging::SecretHeaders;
use thiserror::Error;

use crate::azure_stream_errors::error::{AzureStreamFailure, Reported, StreamingError};
use crate::endpoints::initialize::EndpointError;

pub const MISSING_MODEL_OR_MESSAGES: &str = "Missing model or messages";
pub const MAX_RETRIES_TYPE: &str = "max retries must be an int";
pub const SYNC_CLIENT: &str = "azure_client is not an instance of AzureOpenAI or OpenAI";
pub const ASYNC_CLIENT: &str = "Azure client is not an instance of AsyncAzureOpenAI or AsyncOpenAI";
pub const STRING_RESPONSE: &str = "Unexpected string response from Azure";
pub const PREVIEW_CHARS: usize = 500;

// One failure as AzureChatCompletion.completion reports it: an AzureOpenAIError, a source exception acompletion re-raises, or a cancelled call
#[derive(Debug, Error)]
pub enum AzureChatError {
    #[error("{message}")]
    Azure {
        status: u16,
        message: String,
        headers: Option<SecretHeaders>,
        body: Option<Json>,
    },
    // acompletion re-raises an exception that carries its own status_code unchanged
    #[error("{message}")]
    Source {
        status: Option<u16>,
        message: String,
        headers: Option<SecretHeaders>,
        body: Option<Json>,
    },
    #[error("the request was cancelled")]
    Cancelled,
}

impl AzureChatError {
    #[must_use]
    pub fn azure(status: u16, message: String) -> Self {
        Self::Azure {
            status,
            message,
            headers: None,
            body: None,
        }
    }

    // getattr(error, "status_code", 500), headers and body off a failure the send raised
    #[must_use]
    pub fn wrapped(source: &RawChatError) -> Self {
        match source {
            RawChatError::Status(status) => Self::Azure {
                status: status.status,
                message: status.message.clone(),
                headers: Some(SecretHeaders::from_response(&status.headers)),
                body: Some(serde_json::from_value(status.body.clone()).unwrap_or(Json::Null)),
            },
            RawChatError::BadRequest(reply) => Self::Azure {
                status: 400,
                message: reply.message.clone(),
                headers: Some(SecretHeaders::from_response(&reply.headers)),
                body: Some(serde_json::from_value(reply.body.clone()).unwrap_or(Json::Null)),
            },
            RawChatError::Timeout { .. }
            | RawChatError::Other { .. }
            | RawChatError::Count { .. }
            | RawChatError::Encode { .. } => Self::azure(500, source.to_string()),
        }
    }

    // the async wrapper re-raises a status-bearing exception and wraps the rest into a 500
    #[must_use]
    pub fn async_failure(source: &RawChatError) -> Self {
        match source {
            RawChatError::Status(status) => Self::Source {
                status: Some(status.status),
                message: status.message.clone(),
                headers: Some(SecretHeaders::from_response(&status.headers)),
                body: Some(serde_json::from_value(status.body.clone()).unwrap_or(Json::Null)),
            },
            RawChatError::BadRequest(reply) => Self::Source {
                status: Some(400),
                message: reply.message.clone(),
                headers: Some(SecretHeaders::from_response(&reply.headers)),
                body: Some(serde_json::from_value(reply.body.clone()).unwrap_or(Json::Null)),
            },
            RawChatError::Timeout { .. }
            | RawChatError::Other { .. }
            | RawChatError::Count { .. }
            | RawChatError::Encode { .. } => Self::azure(500, source.to_string()),
        }
    }

    #[must_use]
    pub fn from_endpoint(error: &EndpointError) -> Self {
        Self::azure(error.status().unwrap_or(500), error.to_string())
    }

    // the outer try re-raises an AzureOpenAIError and wraps every other exception with its status
    #[must_use]
    pub fn from_streaming(error: &StreamingError) -> Self {
        match error {
            StreamingError::Azure(failure) => Self::from_stream_failure(failure),
            StreamingError::Logging(_) | StreamingError::Chunk(_) | StreamingError::Usage(_) => {
                Self::azure(500, error.to_string())
            }
        }
    }

    fn from_stream_failure(failure: &AzureStreamFailure) -> Self {
        let status = match &failure.status {
            Reported::Absent => 500,
            Reported::Present(value) => value.unwrap_or(500),
        };
        Self::Azure {
            status,
            message: failure.message().to_owned(),
            headers: failure.headers.clone(),
            body: failure.body.clone(),
        }
    }

    // acompletion's status-bearing rethrow becomes the AzureOpenAIError the outer completion raises
    #[must_use]
    pub fn wrap_source(self) -> Self {
        match self {
            Self::Source {
                status,
                message,
                headers,
                body,
            } => Self::Azure {
                status: status.unwrap_or(500),
                message,
                headers,
                body,
            },
            Self::Azure { .. } | Self::Cancelled => self,
        }
    }

    // str(exception), which the async except blocks log as the original response
    #[must_use]
    pub fn text(&self) -> String {
        match self {
            Self::Azure { message, .. } | Self::Source { message, .. } => message.clone(),
            Self::Cancelled => String::new(),
        }
    }
}

// f"Unexpected string response from Azure: {response[:500]}", sliced by characters like Python
#[must_use]
pub fn string_response(text: &str) -> String {
    let preview: String = text.chars().take(PREVIEW_CHARS).collect();
    format!("{STRING_RESPONSE}: {preview}")
}
