use betterlite_http::proxy::transport_error::TransportError;
use http::header::InvalidHeaderValue;
use thiserror::Error;
use url::ParseError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyFault {
    Malformed,
    Truncated,
    Shape,
}

#[derive(Debug, Error)]
pub enum ThreadsError {
    #[error("the api_key client option must be set")]
    MissingApiKey,
    #[error("api_base is not a valid URL: {source}")]
    BaseUrl {
        #[source]
        source: ParseError,
    },
    #[error("a credential is not a valid header value: {source}")]
    Header {
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("expected a non-empty value for thread_id")]
    EmptyThreadId,
    #[error("thread_id would form the dot segment {segment}")]
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
    #[error("the response could not be decoded: {source}")]
    Reply {
        fault: ReplyFault,
        #[source]
        source: serde_json::Error,
    },
}
