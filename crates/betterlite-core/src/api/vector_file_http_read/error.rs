use std::error::Error as StdError;

use betterlite_logging::logging_pre_post::error::PrePostError;
use http::header::{InvalidHeaderName, InvalidHeaderValue};
use thiserror::Error;

use crate::api::vector_store_http_write::error::{BaseLlmException, ClientError};

// A provider URL builder or request transform that refuses the request before anything is sent
#[derive(Debug, Error)]
#[error(transparent)]
pub struct PrepareFailure {
    pub source: Box<dyn StdError + Send + Sync>,
}

#[derive(Debug, Error)]
pub enum ReadError {
    #[error(transparent)]
    Provider(#[from] BaseLlmException),
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error(transparent)]
    Logging(#[from] PrePostError),
    #[error(transparent)]
    Prepare(#[from] PrepareFailure),
    #[error("the request URL {url} is not a URL")]
    RequestUrl {
        url: String,
        #[source]
        source: url::ParseError,
    },
    #[error("the {name} header name is not a header name")]
    HeaderName {
        name: String,
        #[source]
        source: InvalidHeaderName,
    },
    #[error("the {name} header value is not a header value")]
    HeaderValue {
        name: String,
        #[source]
        source: InvalidHeaderValue,
    },
}
