use reqwest::StatusCode;
use reqwest::header::InvalidHeaderValue;
use thiserror::Error;

use crate::generic_guardrail_contracts::GuardrailExtraValue;

// LiteLLM's detail["error"] for a blocked call
pub const GRAYSWAN_BLOCK_ERROR_MSG: &str = "Blocked by Gray Swan Guardrail";

#[derive(Debug, Error)]
pub enum GraySwanFailure {
    #[error("the Gray Swan API request timed out")]
    Timeout { source: reqwest::Error },
    #[error("the Gray Swan API could not be reached")]
    Request { source: reqwest::Error },
    #[error("the Gray Swan API returned status {status}")]
    HttpStatus { status: StatusCode },
    #[error("the Gray Swan API key is not a valid header")]
    InvalidHeader { source: InvalidHeaderValue },
    #[error("the Gray Swan API response is larger than {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("the Gray Swan API response is not valid JSON")]
    ResponseJson { source: serde_json::Error },
    #[error("{reason}")]
    Blocked {
        reason: String,
        violation_location: &'static str,
        violation: f64,
        violated_rules: Vec<GuardrailExtraValue>,
        mutation: Option<bool>,
        ipi: Option<bool>,
    },
}

impl GraySwanFailure {
    #[must_use]
    pub const fn http_status(&self) -> StatusCode {
        match self {
            Self::Timeout { .. } => StatusCode::REQUEST_TIMEOUT,
            Self::HttpStatus { status } => {
                if matches!(status.as_u16(), 400..=599) {
                    *status
                } else {
                    StatusCode::INTERNAL_SERVER_ERROR
                }
            }
            Self::Blocked { .. } => StatusCode::BAD_REQUEST,
            Self::Request { .. }
            | Self::InvalidHeader { .. }
            | Self::ResponseTooLarge { .. }
            | Self::ResponseJson { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    // LiteLLM's getattr(exc, "status_code", None): only an HTTP error or a block carries one
    #[must_use]
    pub const fn carried_status(&self) -> Option<u16> {
        match self {
            Self::HttpStatus { status } => Some(status.as_u16()),
            Self::Blocked { .. } => Some(StatusCode::BAD_REQUEST.as_u16()),
            Self::Timeout { .. }
            | Self::Request { .. }
            | Self::InvalidHeader { .. }
            | Self::ResponseTooLarge { .. }
            | Self::ResponseJson { .. } => None,
        }
    }
}

// LiteLLM's GraySwanGuardrailAPIError: a message plus the status code of the error it wrapped
#[derive(Debug, Error)]
#[error("{message}")]
pub struct GraySwanGuardrailAPIError {
    pub message: String,
    pub status_code: Option<u16>,
    #[source]
    pub cause: Option<GraySwanFailure>,
}

impl GraySwanGuardrailAPIError {
    #[must_use]
    pub const fn new(message: String, status_code: Option<u16>) -> Self {
        Self {
            message,
            status_code,
            cause: None,
        }
    }

    #[must_use]
    pub fn from_failure(failure: GraySwanFailure) -> Self {
        Self {
            message: failure.to_string(),
            status_code: failure.carried_status(),
            cause: Some(failure),
        }
    }

    #[must_use]
    pub fn http_status(&self) -> StatusCode {
        self.cause.as_ref().map_or(
            StatusCode::INTERNAL_SERVER_ERROR,
            GraySwanFailure::http_status,
        )
    }
}

impl From<GraySwanFailure> for GraySwanGuardrailAPIError {
    fn from(failure: GraySwanFailure) -> Self {
        Self::from_failure(failure)
    }
}
