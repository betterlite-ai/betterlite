use betterlite_http::error::HttpError;
use thiserror::Error;

use crate::agent_requests::error::AgentRequestError;

// AzureAIAgentsError is a BaseLLMException: a status and a message, with the headers get_error_class receives dropped
#[derive(Debug, Error)]
pub enum AgentResultError {
    #[error("Streaming request failed: {body}")]
    StreamFailed { status: u16, body: String },
    #[error("Streaming request failed: the error body is not UTF-8")]
    StreamFailedNotText { status: u16 },
    #[error("the streaming response body could not be read")]
    Body(#[from] HttpError),
    #[error("the streaming response body is larger than {limit} bytes")]
    BodyTooLarge { limit: usize },
    #[error("a streaming response line is longer than {limit} bytes")]
    LineTooLong { limit: usize },
    #[error(transparent)]
    Request(#[from] AgentRequestError),
}

impl AgentResultError {
    // the status Python's BaseLLMException carries; a transport failure has none
    #[must_use]
    pub const fn status(&self) -> Option<u16> {
        match self {
            Self::StreamFailed { status, .. } | Self::StreamFailedNotText { status } => {
                Some(*status)
            }
            Self::Body(_) | Self::BodyTooLarge { .. } | Self::LineTooLong { .. } => None,
            Self::Request(error) => error.status(),
        }
    }
}
