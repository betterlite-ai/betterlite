use reqwest::StatusCode;
use reqwest::header::InvalidHeaderValue;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AliceError {
    #[error(
        "Alice API key is required. Set the `ALICE_API_KEY` environment variable or pass `api_key` in the guardrail config."
    )]
    MissingSecrets,
    #[error("the af-api-key header value is not valid")]
    InvalidApiKeyHeader { source: InvalidHeaderValue },
    #[error("the Alice evaluation did not answer in time")]
    Timeout { source: reqwest::Error },
    #[error("could not reach the Alice evaluation endpoint")]
    Request { source: reqwest::Error },
    #[error("the Alice evaluation endpoint answered with status {status}")]
    Status { status: StatusCode },
    #[error("the Alice evaluation answer is larger than {limit} bytes")]
    ResponseTooLarge { limit: usize },
    #[error("the Alice evaluation answer is not valid JSON")]
    ResponseJson { source: serde_json::Error },
    #[error("the Alice evaluation answer is not a JSON object")]
    NonObjectBody,
    #[error("the Alice verdict does not have the declared shape")]
    MalformedVerdict { source: serde_json::Error },
    #[error("unrecognized verdict: {verdict:?}")]
    UnrecognizedVerdict { verdict: Option<String> },
    #[error("the guardrail inputs could not be serialized")]
    Payload { source: serde_json::Error },
}

#[must_use]
pub fn request_error(source: reqwest::Error) -> AliceError {
    if source.is_timeout() {
        AliceError::Timeout { source }
    } else {
        AliceError::Request { source }
    }
}
