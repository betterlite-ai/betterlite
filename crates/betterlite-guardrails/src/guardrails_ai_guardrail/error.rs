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
