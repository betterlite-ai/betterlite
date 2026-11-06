use betterlite_http::proxy::transport_error::TransportError;
use http::header::InvalidHeaderValue;
use thiserror::Error;
use url::ParseError;

use crate::assistants::error::AssistantsError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyFault {
    Malformed,
    Truncated,
    Shape,
}

#[derive(Debug, Error)]
pub enum ThreadsError {
    #[error("the api_key client option must be set")]
    MissingApiKey,
    #[error("api_base is not a valid URL: {source}")]
    BaseUrl {
        #[source]
        source: ParseError,
    },
    #[error("a credential is not a valid header value: {source}")]
    Header {
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("expected a non-empty value for thread_id")]
    EmptyThreadId,
    #[error("thread_id would form the dot segment {segment}")]
    DotSegment { segment: String },
    #[error("the request could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the provider could not be reached: {source}")]
    Transport {
        #[source]
        source: TransportError,
    },
    #[error("the provider answered with status {status}")]
    Status { status: u16, body: Vec<u8> },
    #[error("the response could not be decoded: {source}")]
    Reply {
        fault: ReplyFault,
        #[source]
        source: serde_json::Error,
    },
}

impl From<AssistantsError> for ThreadsError {
    fn from(error: AssistantsError) -> Self {
        match error {
            AssistantsError::MissingApiKey => Self::MissingApiKey,
            AssistantsError::BaseUrl { source } => Self::BaseUrl { source },
            AssistantsError::Header { source } => Self::Header { source },
            AssistantsError::EmptyAssistantId => Self::EmptyThreadId,
            AssistantsError::DotSegment { segment } => Self::DotSegment { segment },
            AssistantsError::Encode { source } => Self::Encode { source },
            AssistantsError::Transport { source } => Self::Transport { source },
            AssistantsError::Status { status, body } => Self::Status { status, body },
            AssistantsError::Malformed { source } => Self::Reply {
                fault: ReplyFault::Malformed,
                source,
            },
            AssistantsError::Truncated { source } => Self::Reply {
                fault: ReplyFault::Truncated,
                source,
            },
            AssistantsError::Shape { source } => Self::Reply {
                fault: ReplyFault::Shape,
                source,
            },
        }
    }
}
