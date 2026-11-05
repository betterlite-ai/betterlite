use betterlite_http::proxy::transport_error::TransportError;
use http::header::InvalidHeaderValue;
use serde_json::error::Category;
use thiserror::Error;
use url::ParseError;

#[derive(Debug, Error)]
pub enum AssistantsError {
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
    #[error("expected a non-empty value for assistant_id")]
    EmptyAssistantId,
    #[error("assistant_id would form the dot segment {segment}")]
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
}

impl From<serde_json::Error> for AssistantsError {
    fn from(source: serde_json::Error) -> Self {
        match source.classify() {
            Category::Eof => Self::Truncated { source },
            Category::Data => Self::Shape { source },
            Category::Syntax | Category::Io => Self::Malformed { source },
        }
    }
}
