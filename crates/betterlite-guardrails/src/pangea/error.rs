use reqwest::StatusCode;
use serde::Serialize;
use thiserror::Error;

pub const BLOCKED_MESSAGE: &str = "Violated Pangea guardrail policy";
const BACKEND_MESSAGE: &str = "Error in Pangea Guardrail";
const MISSING_KEY_MESSAGE: &str = "Pangea API Key not found. Set PANGEA_API_KEY environment variable or pass it in litellm_params.";

#[derive(Debug, Error)]
pub enum PangeaGuardrailError {
    #[error("Pangea guardrail name is required")]
    MissingGuardrailName,
    // LiteLLM raises its PangeaGuardrailMissingSecrets exception here instead.
    #[error("{MISSING_KEY_MESSAGE}")]
    MissingSecrets,
    #[error("the Pangea AI Guard request failed")]
    Request { source: reqwest::Error },
    #[error("the Pangea AI Guard answered with status {status}")]
    HttpStatus { status: StatusCode },
    #[error("the Pangea AI Guard answer is larger than {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("the Pangea AI Guard answer is not valid JSON")]
    ResponseJson { source: serde_json::Error },
    #[error("{BLOCKED_MESSAGE}")]
    Blocked,
    #[error("the Pangea AI Guard answer has no {field} to transform")]
    MissingData { field: &'static str },
    #[error("a transformed text completion needs exactly one message with content")]
    InvalidTextCompletion,
}

impl PangeaGuardrailError {
    // LiteLLM raises HTTPException(status_code=400) for a block and wraps every other failure in a 500.
    #[must_use]
    pub const fn status_code(&self) -> StatusCode {
        if matches!(self, Self::Blocked) {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }

    #[must_use]
    pub fn detail(&self, guardrail_name: &str) -> PangeaErrorDetail {
        let blocked = matches!(self, Self::Blocked);
        let message = if blocked {
            BLOCKED_MESSAGE
        } else {
            BACKEND_MESSAGE
        };
        PangeaErrorDetail {
            error: message,
            guardrail_name: guardrail_name.to_owned(),
            exceptions: (!blocked).then(|| self.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PangeaErrorDetail {
    pub error: &'static str,
    pub guardrail_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exceptions: Option<String>,
}
