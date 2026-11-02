use reqwest::StatusCode;
use reqwest::header::InvalidHeaderValue;
use thiserror::Error;

use super::wire::JavelinGuardResponse;

#[derive(Debug, Error)]
pub enum JavelinError {
    #[error("the x-javelin-apikey header value is not valid")]
    InvalidApiKeyHeader { source: InvalidHeaderValue },
    #[error("the x-javelin-application header value is not valid")]
    InvalidApplicationHeader { source: InvalidHeaderValue },
    #[error("the Javelin guard API did not answer in time")]
    Timeout { source: reqwest::Error },
    #[error("could not reach the Javelin guard API")]
    Request { source: reqwest::Error },
    #[error("the Javelin guard API answered with status {status}")]
    HttpStatus { status: StatusCode },
    #[error("the Javelin guard API answer is larger than {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("the Javelin guard API answer is not valid JSON")]
    ResponseJson { source: serde_json::Error },
    #[error("Javelin blocked the request: {reject_prompt}")]
    Rejected {
        reject_prompt: String,
        javelin_guardrail_response: JavelinGuardResponse,
    },
}

impl JavelinError {
    #[must_use]
    pub const fn status_code(&self) -> StatusCode {
        // LiteLLM raises HTTPException(status_code=500, ...) on a rejection; every other variant is swallowed by the fail-open wrapper and never reaches a caller.
        match self {
            Self::InvalidApiKeyHeader { .. }
            | Self::InvalidApplicationHeader { .. }
            | Self::Timeout { .. }
            | Self::Request { .. }
            | Self::HttpStatus { .. }
            | Self::ResponseTooLarge { .. }
            | Self::ResponseJson { .. }
            | Self::Rejected { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

#[derive(Debug, Error)]
pub enum JavelinInitializationError {
    #[error(
        "JavelinGuardrailException: litellm_params.guard_name is required to initialize the Javelin guardrail"
    )]
    MissingGuardName,
}
