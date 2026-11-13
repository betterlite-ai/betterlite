use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::url_policy::error::UrlPolicyError;
use thiserror::Error;

use crate::image_transport::decode::{ImageDecodeError, ImageOperation};
use crate::image_transport::json::JsonValue;

#[derive(Debug, Error)]
pub enum ImageTransportError {
    #[error("Rejected polling URL: {source}")]
    RejectedPollingUrl {
        #[source]
        source: UrlPolicyError,
    },
    #[error("{body}")]
    MissingOperationLocation { body: String },
    #[error("Polling response missing 'status' field")]
    MissingStatusInitial,
    #[error("Operation polling timed out.")]
    PollingTimeout,
    #[error("{message}")]
    OperationFailed { message: String, body: JsonValue },
    #[error("the image operation response is not JSON")]
    Decode(#[from] ImageDecodeError),
    #[error("the polling response has no status")]
    MissingStatus,
    #[error("the succeeded operation has no result")]
    MissingResult,
    #[error("the retry-after header is not an integer: {value}")]
    RetryAfter { value: String },
    #[error("sleep length must be non-negative: {seconds}")]
    NegativeSleep { seconds: i64 },
    #[error("the image request URL is not valid")]
    Url(#[from] url::ParseError),
    #[error("the image request body cannot be encoded as JSON")]
    Encode(#[from] serde_json::Error),
    #[error(transparent)]
    Transport(#[from] TransportError),
}

impl ImageTransportError {
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::RejectedPollingUrl { .. } | Self::MissingStatusInitial => Some(502),
            Self::MissingOperationLocation { .. } => Some(500),
            Self::PollingTimeout => Some(408),
            Self::OperationFailed { .. } => Some(400),
            Self::Decode(_)
            | Self::MissingStatus
            | Self::MissingResult
            | Self::RetryAfter { .. }
            | Self::NegativeSleep { .. }
            | Self::Url(_)
            | Self::Encode(_)
            | Self::Transport(_) => None,
        }
    }

    pub(crate) fn operation_failed(operation: ImageOperation) -> Self {
        let message = FailureMessage::of(&operation.json).render(&operation.json);
        Self::OperationFailed {
            message,
            body: operation.json,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureMessage {
    ErrorMessage,
    ImageGenerationFailed,
    OperationJson,
}

impl FailureMessage {
    #[must_use]
    pub fn of(operation: &JsonValue) -> Self {
        match operation.get("error").unwrap_or(operation) {
            JsonValue::Object(message) if message.contains_key("message") => Self::ErrorMessage,
            JsonValue::Object(_) => Self::ImageGenerationFailed,
            JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::String(_)
            | JsonValue::Array(_) => Self::OperationJson,
        }
    }

    #[must_use]
    pub fn render(self, operation: &JsonValue) -> String {
        match self {
            Self::ErrorMessage => error_message(operation),
            Self::ImageGenerationFailed => "Image generation failed".to_owned(),
            Self::OperationJson => serde_json::to_string(operation).unwrap_or_default(),
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ErrorMessage => "error_message",
            Self::ImageGenerationFailed => "image_generation_failed",
            Self::OperationJson => "operation_json",
        }
    }
}

fn error_message(operation: &JsonValue) -> String {
    match operation.get("error").unwrap_or(operation).get("message") {
        Some(JsonValue::String(message)) => message.clone(),
        Some(other) => serde_json::to_string(other).unwrap_or_default(),
        None => "Image generation failed".to_owned(),
    }
}
