use reqwest::StatusCode;
use reqwest::header::InvalidHeaderValue;
use thiserror::Error;

use super::AporiaApiResponse;

#[derive(Debug, Error)]
pub enum AporiaGuardrailError {
    #[error("the {name} environment variable is not set")]
    MissingEnvironment { name: &'static str },
    #[error("the Aporia API request timed out")]
    Timeout { source: reqwest::Error },
    #[error("the Aporia API could not be reached")]
    Request { source: reqwest::Error },
    #[error("the Aporia API key is not a valid header value")]
    InvalidHeader { source: InvalidHeaderValue },
    #[error("the Aporia API returned status {status}")]
    HttpStatus { status: StatusCode },
    #[error("the Aporia API response is larger than {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("the Aporia API response is not valid JSON")]
    ResponseJson { source: serde_json::Error },
    #[error("Violated guardrail policy")]
    Blocked { response: AporiaApiResponse },
}

impl AporiaGuardrailError {
    // LiteLLM raises HTTPException(400, ...) on a block and the upstream status on a non-2xx reply.
    #[must_use]
    pub const fn status_code(&self) -> StatusCode {
        match self {
            Self::HttpStatus { status } => *status,
            Self::Blocked { .. } => StatusCode::BAD_REQUEST,
            Self::MissingEnvironment { .. }
            | Self::Timeout { .. }
            | Self::Request { .. }
            | Self::InvalidHeader { .. }
            | Self::ResponseTooLarge { .. }
            | Self::ResponseJson { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}
