use std::error::Error as StdError;
use std::string::FromUtf8Error;

use http::header::InvalidHeaderValue;
use thiserror::Error;

use super::payload::PayloadError;

#[derive(Debug, Error)]
pub enum BraintrustError<E>
where
    E: StdError + 'static,
{
    #[error("the Braintrust request body could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error("the Braintrust request failed")]
    Transport(#[source] E),
    #[error("Braintrust answered with status {status}: {body}")]
    Status { status: u16, body: String },
    #[error("the Braintrust response body is not UTF-8")]
    Body(#[source] FromUtf8Error),
    #[error("the Braintrust response body is not JSON")]
    Response(#[source] serde_json::Error),
    #[error("the Braintrust response has no string {field}")]
    ProjectField { field: &'static str },
    #[error("the Braintrust URL {url} is not valid")]
    Url {
        url: String,
        #[source]
        source: url::ParseError,
    },
    #[error("the Braintrust Authorization header is not valid")]
    Header(#[source] InvalidHeaderValue),
    #[error("the Braintrust project cache is poisoned")]
    CachePoisoned,
    #[error(transparent)]
    Payload(#[from] PayloadError),
}
