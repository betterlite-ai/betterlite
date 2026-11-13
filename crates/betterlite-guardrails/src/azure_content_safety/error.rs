use reqwest::StatusCode;
use reqwest::header::InvalidHeaderValue;
use serde::Serialize;
use thiserror::Error;

use super::legacy_wire::TextCategory;

pub const VIOLATION_MESSAGE: &str = "Violated content safety policy";

#[derive(Debug, Error)]
pub enum AzureContentSafetyError {
    #[error("the Azure Content Safety API key cannot be sent as a header value")]
    InvalidApiKey { source: InvalidHeaderValue },
    #[error("the Azure Content Safety request timed out")]
    Timeout { source: reqwest::Error },
    #[error("the Azure Content Safety API could not be reached")]
    Request { source: reqwest::Error },
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
}

#[derive(Debug, Error)]
pub enum LegacyAzureContentSafetyError {
    #[error(
        "the Azure Content Safety category `{category}` is not one of Hate, SelfHarm, Sexual or Violence"
    )]
    UnknownCategory { category: String },
    #[error("the Azure Content Safety API key cannot be sent as a header value")]
    InvalidApiKey { source: InvalidHeaderValue },
    #[error("the Azure Content Safety request timed out")]
    Timeout { source: reqwest::Error },
    #[error("the Azure Content Safety API could not be reached")]
    Request { source: reqwest::Error },
    #[error("the Azure Content Safety API answered with status {status}")]
    HttpStatus { status: StatusCode },
    #[error("the Azure Content Safety response is larger than {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("the Azure Content Safety response is not valid JSON")]
    ResponseJson { source: serde_json::Error },
    #[error("Violated content safety policy")]
    Violation {
        source: Option<String>,
        category: TextCategory,
        severity: i64,
    },
}

impl LegacyAzureContentSafetyError {
    // LiteLLM raises HTTPException(400) for a violation and lets every SDK failure surface as a 500.
    #[must_use]
    pub const fn status_code(&self) -> StatusCode {
        match self {
            Self::Violation { .. } => StatusCode::BAD_REQUEST,
            Self::UnknownCategory { .. }
            | Self::InvalidApiKey { .. }
            | Self::Timeout { .. }
            | Self::Request { .. }
            | Self::HttpStatus { .. }
            | Self::ResponseTooLarge { .. }
            | Self::ResponseJson { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    // The dict LiteLLM puts in HTTPException.detail; the other variants have none.
    #[must_use]
    pub fn detail(&self) -> Option<LegacyViolationDetail<'_>> {
        match self {
            Self::Violation {
                source,
                category,
                severity,
            } => Some(LegacyViolationDetail {
                error: VIOLATION_MESSAGE,
                source: source.as_deref(),
                category: *category,
                severity: *severity,
            }),
            Self::UnknownCategory { .. }
            | Self::InvalidApiKey { .. }
            | Self::Timeout { .. }
            | Self::Request { .. }
            | Self::HttpStatus { .. }
            | Self::ResponseTooLarge { .. }
            | Self::ResponseJson { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LegacyViolationDetail<'detail> {
    pub error: &'static str,
    pub source: Option<&'detail str>,
    pub category: TextCategory,
    pub severity: i64,
}
