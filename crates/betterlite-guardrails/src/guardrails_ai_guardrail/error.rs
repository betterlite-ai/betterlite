use std::string::FromUtf8Error;

use reqwest::StatusCode;
use thiserror::Error;

use super::GuardrailsAiApiResponse;

#[derive(Debug, Error)]
pub enum GuardrailsAiGuardrailError {
    #[error(
        "GuardrailsAIException - Please pass the Guardrails AI guard name via 'litellm_params::guard_name'"
    )]
    MissingGuardName,
    #[error("the configured Guardrails AI API base is not a valid URL")]
    InvalidApiBase { source: url::ParseError },
    #[error("the Guardrails AI API request timed out")]
    Timeout { source: reqwest::Error },
    #[error("the Guardrails AI API could not be reached")]
    Request { source: reqwest::Error },
    #[error("the Guardrails AI API returned status {status}")]
    HttpStatus { status: StatusCode },
    #[error("the Guardrails AI API response is larger than {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("the Guardrails AI API response is not valid JSON")]
    ResponseJson { source: serde_json::Error },
    #[error("Violated guardrail policy")]
    Blocked { response: GuardrailsAiApiResponse },
    #[error("Violated guardrail policy")]
    PreCallBlocked { response: String },
    #[error("the Guardrails AI pre-call response has no output data")]
    PreCallMissingOutput,
    #[error("the Guardrails AI pre-call response is not valid UTF-8")]
    InvalidUtf8 { source: FromUtf8Error },
}

impl GuardrailsAiGuardrailError {
    // LiteLLM raises HTTPException(400, ...) on a block and the upstream status on a non-2xx reply.
    #[must_use]
    pub const fn status_code(&self) -> StatusCode {
        match self {
            Self::HttpStatus { status } => *status,
            Self::Blocked { .. } | Self::PreCallBlocked { .. } => StatusCode::BAD_REQUEST,
            Self::MissingGuardName
            | Self::InvalidApiBase { .. }
            | Self::Timeout { .. }
            | Self::Request { .. }
            | Self::ResponseTooLarge { .. }
            | Self::ResponseJson { .. }
            | Self::PreCallMissingOutput
            | Self::InvalidUtf8 { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}
