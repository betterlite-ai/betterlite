use reqwest::StatusCode;
use reqwest::header::InvalidHeaderValue;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DynamoAiError {
    #[error("the DynamoAI API request timed out")]
    Timeout { source: reqwest::Error },
    #[error("the DynamoAI API could not be reached")]
    Request { source: reqwest::Error },
    #[error("the DynamoAI API returned status {status}")]
    HttpStatus { status: StatusCode },
    #[error("the DynamoAI API key is not a valid header value")]
    InvalidHeader { source: InvalidHeaderValue },
    #[error("the DynamoAI API response is larger than {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("the DynamoAI API response is not valid JSON")]
    ResponseJson { source: serde_json::Error },
    #[error("{message}")]
    Blocked { message: String },
}
