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
    // LiteLLM's getattr(exc, "status_code", None) beside the status its handler answers with
    #[must_use]
    pub const fn statuses(&self) -> (StatusCode, Option<u16>) {
        match self {
            Self::Timeout { .. } => (
                StatusCode::REQUEST_TIMEOUT,
                Some(StatusCode::REQUEST_TIMEOUT.as_u16()),
            ),
            Self::HttpStatus { status } => (*status, Some(status.as_u16())),
            Self::Blocked { .. } => (
                StatusCode::BAD_REQUEST,
                Some(StatusCode::BAD_REQUEST.as_u16()),
            ),
            Self::Request { .. }
            | Self::InvalidHeader { .. }
            | Self::ResponseTooLarge { .. }
            | Self::ResponseJson { .. } => (StatusCode::INTERNAL_SERVER_ERROR, None),
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
    pub fn from_failure(failure: GraySwanFailure) -> Self {
        let status_code = failure.statuses().1;
        Self {
            message: failure.to_string(),
            status_code,
            cause: Some(failure),
        }
    }

    #[must_use]
    pub const fn http_status(&self) -> StatusCode {
        let Some(failure) = &self.cause else {
            return StatusCode::INTERNAL_SERVER_ERROR;
        };
        failure.statuses().0
    }
}

impl From<GraySwanFailure> for GraySwanGuardrailAPIError {
    fn from(failure: GraySwanFailure) -> Self {
        Self::from_failure(failure)
    }
}
