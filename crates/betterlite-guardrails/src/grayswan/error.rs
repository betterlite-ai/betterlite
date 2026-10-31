use reqwest::StatusCode;
use reqwest::header::InvalidHeaderValue;
use thiserror::Error;

use crate::generic_guardrail_contracts::GuardrailExtraValue;

#[derive(Debug, Error)]
pub enum GraySwanGuardrailAPIError {
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
    #[error("Gray Swan flagged content{reason}")]
    Blocked {
        reason: String,
        violation_location: &'static str,
        violation: f64,
        violated_rules: Vec<GuardrailExtraValue>,
        mutation: Option<bool>,
        ipi: Option<bool>,
    },
}
impl GraySwanGuardrailAPIError {
    #[must_use]
    pub const fn status_code(&self) -> StatusCode {
        match self {
            Self::Timeout { .. }
            | Self::Request { .. }
            | Self::HttpStatus { .. }
            | Self::InvalidHeader { .. }
            | Self::ResponseTooLarge { .. }
            | Self::ResponseJson { .. }
            | Self::Blocked { .. } => StatusCode::BAD_REQUEST,
        }
    }
}
