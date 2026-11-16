use std::error::Error as StdError;

use betterlite_http::proxy::transport_error::TransportError;
use betterlite_openai::aleph_alpha::error::AlephAlphaError;
use http::HeaderMap;
use thiserror::Error;

#[derive(Debug)]
pub struct StatusFailure {
    pub status_code: u16,
    pub body: Vec<u8>,
    pub exception_headers: Option<HeaderMap>,
    // getattr(e.response, "headers", None): the fallback make_call prefers whenever the first is None
    pub response_headers: Option<HeaderMap>,
}

#[derive(Debug)]
pub enum PostFailure<Failure> {
    Status(Box<StatusFailure>),
    Known(Failure),
    Unknown(Failure),
}

#[derive(Debug, Error)]
pub enum StreamFailure {
    #[error(transparent)]
    Transport(TransportError),
    #[error(transparent)]
    Lines(AlephAlphaError),
}

#[derive(Debug, Error)]
pub enum StreamOpenError<Failure>
where
    Failure: StdError + Send + Sync + 'static,
{
    #[error("{message}")]
    Anthropic {
        status_code: u16,
        message: String,
        headers: Option<HeaderMap>,
    },
    #[error("{source}")]
    Reraised {
        #[source]
        source: Failure,
    },
    #[error("{source}")]
    Unknown {
        #[source]
        source: Failure,
    },
    // betterlite's typed error where LiteLLM hands httpx's invalid-url exception to the 500 branch
    #[error("the api_base is not a URL")]
    Endpoint {
        #[source]
        source: url::ParseError,
    },
}
