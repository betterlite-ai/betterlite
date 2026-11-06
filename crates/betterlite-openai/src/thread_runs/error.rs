use std::num::ParseIntError;

use betterlite_http::proxy::transport_error::TransportError;
use http::header::{InvalidHeaderValue, ToStrError};
use thiserror::Error;
use url::ParseError;

#[derive(Debug, Error)]
pub enum ThreadRunsError {
    #[error("the api_key client option must be set")]
    MissingApiKey,
    #[error("api_base is not a valid URL: {source}")]
    BaseUrl {
        #[source]
        source: ParseError,
    },
    #[error("api_base cannot carry path segments")]
    BaseUrlPath,
    #[error("a credential is not a valid header value: {source}")]
    Header {
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("expected a non-empty value for thread_id")]
    EmptyThreadId,
    #[error("expected a non-empty value for run_id")]
    EmptyRunId,
    #[error("the path segment {segment} is a dot segment")]
    DotSegment { segment: String },
    #[error("the request could not be written as JSON: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the provider could not be reached: {source}")]
    Transport {
        #[source]
        source: TransportError,
    },
    #[error("the provider answered with status {status}")]
    Status { status: u16, body: Vec<u8> },
    #[error("the response is not JSON: {source}")]
    Malformed {
        #[source]
        source: serde_json::Error,
    },
    #[error("the response ends before the JSON does: {source}")]
    Truncated {
        #[source]
        source: serde_json::Error,
    },
    #[error("the response JSON does not describe the expected object: {source}")]
    Shape {
        #[source]
        source: serde_json::Error,
    },
    #[error("the run stream names the unsupported event {event}")]
    UnsupportedEvent { event: String },
    #[error("the poll-after header is not readable: {source}")]
    PollAfterHeader {
        #[source]
        source: ToStrError,
    },
    #[error("the poll-after header is not a whole number of milliseconds: {value}")]
    PollAfter {
        value: String,
        #[source]
        source: ParseIntError,
    },
}
