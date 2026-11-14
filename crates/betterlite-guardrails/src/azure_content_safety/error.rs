use betterlite_http::proxy::limits::ReadError;
use reqwest::StatusCode;
use reqwest::header::InvalidHeaderValue;
use serde::Serialize;
use thiserror::Error;

use super::legacy_wire::TextCategory;

pub const VIOLATION_MESSAGE: &str = "Violated content safety policy";

#[derive(Debug, Error)]
pub enum AzureContentSafetyError {
    #[error(
        "the Azure Content Safety category `{category}` is not one of Hate, SelfHarm, Sexual or Violence"
    )]
    UnknownCategory { category: String },
    #[error("the Azure Content Safety API key cannot be sent as a header value")]
    InvalidApiKey { source: InvalidHeaderValue },
    #[error("the Azure Content Safety request timed out")]
    Timeout {
        source: reqwest::Error,
        status: StatusCode,
    },
    #[error("the Azure Content Safety API could not be reached")]
    Request { source: reqwest::Error },
    #[error("the Azure Content Safety API answered with status {status}")]
    HttpStatus { status: StatusCode, body: String },
    #[error("the Azure Content Safety response is larger than {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("the Azure Content Safety response is not valid JSON")]
    ResponseJson { source: serde_json::Error },
    #[error("the `messages` of a {call_type} request must be a list")]
    MessagesShape { call_type: String },
    #[error("an entry of the `{field}` of a {call_type} request is not a message object")]
    MessageJson {
        field: &'static str,
        call_type: String,
        source: serde_json::Error,
    },
    #[error("Violated content safety policy")]
    Violation {
        origin: Option<String>,
        category: TextCategory,
        severity: i64,
    },
}

impl AzureContentSafetyError {
    // LiteLLM raises HTTPException(400) for a violation and copies a provider status between 400 and 599.
    #[must_use]
    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::Violation { .. } => StatusCode::BAD_REQUEST,
            Self::Timeout { status, .. } => *status,
            Self::HttpStatus { status, .. }
                if status.is_client_error() || status.is_server_error() =>
            {
                *status
            }
            Self::UnknownCategory { .. }
            | Self::InvalidApiKey { .. }
            | Self::Request { .. }
            | Self::HttpStatus { .. }
            | Self::ResponseTooLarge { .. }
            | Self::ResponseJson { .. }
            | Self::MessagesShape { .. }
            | Self::MessageJson { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    // The dict LiteLLM puts in HTTPException.detail; the other variants have none.
    #[must_use]
    pub fn detail(&self) -> Option<LegacyViolationDetail<'_>> {
        match self {
            Self::Violation {
                origin,
                category,
                severity,
            } => Some(LegacyViolationDetail {
                error: VIOLATION_MESSAGE,
                source: origin.as_deref(),
                category: *category,
                severity: *severity,
            }),
            Self::UnknownCategory { .. }
            | Self::InvalidApiKey { .. }
            | Self::Timeout { .. }
            | Self::Request { .. }
            | Self::HttpStatus { .. }
            | Self::ResponseTooLarge { .. }
            | Self::ResponseJson { .. }
            | Self::MessagesShape { .. }
            | Self::MessageJson { .. } => None,
        }
    }
}

// The caller names the status its timeout answers with, since httpx and azure-core report it differently.
pub(crate) fn request_error(
    source: reqwest::Error,
    timeout_status: StatusCode,
) -> AzureContentSafetyError {
    if source.is_timeout() {
        AzureContentSafetyError::Timeout {
            source,
            status: timeout_status,
        }
    } else {
        AzureContentSafetyError::Request { source }
    }
}

pub(crate) fn read_error(error: ReadError, timeout_status: StatusCode) -> AzureContentSafetyError {
    match error {
        ReadError::TooLarge { limit } => AzureContentSafetyError::ResponseTooLarge { limit },
        ReadError::Failed(source) => request_error(source, timeout_status),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LegacyViolationDetail<'detail> {
    pub error: &'static str,
    pub source: Option<&'detail str>,
    pub category: TextCategory,
    pub severity: i64,
}
