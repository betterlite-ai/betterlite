use reqwest::StatusCode;
use reqwest::header::InvalidHeaderValue;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum XecGuardError {
    #[error("the XecGuard API request timed out")]
    Timeout { source: reqwest::Error },
    #[error("the XecGuard API could not be reached")]
    Request { source: reqwest::Error },
    #[error("the XecGuard API key is not a valid header")]
    InvalidHeader { source: InvalidHeaderValue },
    #[error("the XecGuard API returned status {status}")]
    HttpStatus { status: StatusCode },
    #[error("the XecGuard API response is larger than {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("the XecGuard API response is not valid JSON")]
    ResponseJson { source: serde_json::Error },
    #[error("the XecGuard payload is not valid JSON")]
    PayloadEncoding { source: serde_json::Error },
    #[error("XecGuard API unreachable (block_on_error=True): {source}")]
    Unreachable { source: Box<Self> },
}
