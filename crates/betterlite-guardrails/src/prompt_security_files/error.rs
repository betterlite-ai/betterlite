use reqwest::StatusCode;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FileSanitizationError {
    #[error(
        "Couldn't get Prompt Security api base or key, either set `PROMPT_SECURITY_API_BASE` and `PROMPT_SECURITY_API_KEY` in the environment or pass them as parameters to the guardrail in the config file"
    )]
    MissingSecrets,
    #[error("Failed to get jobId from Prompt Security")]
    MissingJobId,
    #[error("Unexpected sanitization status: {status}")]
    UnexpectedStatus { status: String },
    #[error("File sanitization timeout")]
    Timeout,
    #[error("{resource} blocked by Prompt Security. Violations: {violations}")]
    Blocked {
        resource: &'static str,
        violations: String,
    },
    #[error("File sanitization failed: {source}")]
    ImageFailed { source: Box<Self> },
    #[error("Document sanitization failed: {source}")]
    DocumentFailed { source: Box<Self> },
    #[error("the {field} field must be a string")]
    InvalidField { field: &'static str },
    #[error("the data URL has no comma before its payload")]
    DataUrlSeparator,
    #[error("the data URL payload is not valid base64")]
    Base64 { source: base64::DecodeError },
    #[error("the Prompt Security API did not answer in time")]
    RequestTimeout { source: reqwest::Error },
    #[error("the Prompt Security request failed")]
    Request { source: reqwest::Error },
    #[error("the Prompt Security API answered with status {status}")]
    HttpStatus { status: StatusCode },
    #[error("the Prompt Security answer is larger than {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("the Prompt Security answer is not valid JSON")]
    ResponseJson { source: serde_json::Error },
    #[error("the upload content type is not valid")]
    UploadMime { source: reqwest::Error },
}

impl FileSanitizationError {
    #[must_use]
    pub const fn status_code(&self) -> StatusCode {
        match self {
            Self::Timeout => StatusCode::REQUEST_TIMEOUT,
            Self::Blocked { .. } | Self::InvalidField { .. } => StatusCode::BAD_REQUEST,
            Self::MissingSecrets
            | Self::MissingJobId
            | Self::UnexpectedStatus { .. }
            | Self::ImageFailed { .. }
            | Self::DocumentFailed { .. }
            | Self::DataUrlSeparator
            | Self::Base64 { .. }
            | Self::RequestTimeout { .. }
            | Self::Request { .. }
            | Self::HttpStatus { .. }
            | Self::ResponseTooLarge { .. }
            | Self::ResponseJson { .. }
            | Self::UploadMime { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    // LiteLLM raises these as HTTPException, so the item processors let them through and wrap every other error as a 500.
    #[must_use]
    pub const fn is_http_exception(&self) -> bool {
        match self {
            Self::MissingJobId
            | Self::UnexpectedStatus { .. }
            | Self::Timeout
            | Self::Blocked { .. }
            | Self::ImageFailed { .. }
            | Self::DocumentFailed { .. }
            | Self::InvalidField { .. } => true,
            Self::MissingSecrets
            | Self::DataUrlSeparator
            | Self::Base64 { .. }
            | Self::RequestTimeout { .. }
            | Self::Request { .. }
            | Self::HttpStatus { .. }
            | Self::ResponseTooLarge { .. }
            | Self::ResponseJson { .. }
            | Self::UploadMime { .. } => false,
        }
    }
}
