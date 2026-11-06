use betterlite_http::proxy::transport_error::TransportError;
use http::header::InvalidHeaderValue;
use thiserror::Error;
use url::ParseError;

#[derive(Debug, Error)]
pub enum ThreadRunsError {
    #[error("the api_key client option must be set")]
    MissingApiKey,
    #[error("api_base is not a valid URL: {0}")]
    BaseUrl(#[source] ParseError),
    #[error("a credential is not a valid header value: {0}")]
    Header(#[source] InvalidHeaderValue),
    #[error("the request could not be written as JSON: {0}")]
    Encode(#[source] serde_json::Error),
    #[error("expected a non-empty value for thread_id")]
    EmptyThreadId,
    #[error("expected a non-empty value for run_id")]
    EmptyRunId,
    #[error("the path segment {0} is a dot segment")]
    DotSegment(String),
    #[error("the provider could not be reached: {0}")]
    Transport(#[source] TransportError),
    #[error("the provider answered with status {0}")]
    Status(u16, Vec<u8>),
    #[error("the response is not JSON: {0}")]
    Malformed(#[source] serde_json::Error),
    #[error("the response ends before the JSON does: {0}")]
    Truncated(#[source] serde_json::Error),
    #[error("the response JSON does not describe the expected object: {0}")]
    Shape(#[source] serde_json::Error),
    #[error("the poll-after header is not a usable number of milliseconds: {0}")]
    PollAfter(String),
    #[error("the provider reported an error inside the run stream")]
    ProviderError { body: Vec<u8> },
    #[error("the provider stream exceeded the frame byte limit")]
    StreamTooLarge,
    #[error("the provider stream timed out")]
    Timeout,
}
