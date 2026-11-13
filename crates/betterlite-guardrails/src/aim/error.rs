use reqwest::StatusCode;
use reqwest::header::InvalidHeaderValue;
use thiserror::Error;

pub const CONTENT_POLICY_VIOLATION: &str = "content_policy_violation";
pub const BAD_REQUEST_CODE: &str = "400";

#[derive(Debug, Error)]
pub enum AimError {
    #[error("the Aim guardrail has no API key: set AIM_API_KEY or configure api_key")]
    MissingSecrets,
    #[error("an Aim header is not a valid header value")]
    InvalidHeader(#[source] InvalidHeaderValue),
    #[error("the Aim request timed out")]
    Timeout(#[source] reqwest::Error),
    #[error("the Aim request failed")]
    Request(#[source] reqwest::Error),
    #[error("the Aim API returned status {0}")]
    HttpStatus(StatusCode),
    #[error("the Aim response is larger than {0} bytes")]
    ResponseTooLarge(usize),
    #[error("the Aim response is not valid JSON")]
    ResponseJson(#[source] serde_json::Error),
    #[error("the Aim request body could not be serialized")]
    RequestJson(#[source] serde_json::Error),
    #[error("{message}")]
    Rejected { message: String, code: &'static str },
    #[error("an Aim stream frame is not valid JSON")]
    StreamJson(#[source] serde_json::Error),
    #[error("the Aim stream closed before it finished")]
    StreamClosed,
    #[error("{message}")]
    StreamBlocked { message: String },
}

impl AimError {
    #[must_use]
    pub fn bad_request(message: &str) -> Self {
        Self::Rejected {
            message: message.to_owned(),
            code: BAD_REQUEST_CODE,
        }
    }
}
