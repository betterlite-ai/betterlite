use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::response_resources::error::ResponseResourceError;
use http::header::{InvalidHeaderName, InvalidHeaderValue};
use thiserror::Error;

use crate::api::vector_store_http_write::error::{BaseLlmException, ClientError};

#[derive(Debug, Error)]
pub enum ReadError {
    #[error(transparent)]
    Provider(#[from] BaseLlmException),
    #[error(transparent)]
    Response(#[from] ResponseResourceError),
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error(transparent)]
    Logging(#[from] PrePostError),
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
    #[error("the {name} query value is not a string or a number")]
    QueryValue { name: String },
}
