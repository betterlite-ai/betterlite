use betterlite_http::error::HttpError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::vector_stores::error::VectorStoreError;
use http::header::{InvalidHeaderName, InvalidHeaderValue};
use thiserror::Error;

use crate::api::vector_store_http_write::error::{BaseLlmException, ClientError, UrlFailure};

// The file write handlers fail before a provider error in five ways, and each names the field that did not fit
#[derive(Debug, Error)]
pub enum FileWriteError {
    #[error(transparent)]
    Provider(#[from] BaseLlmException),
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error(transparent)]
    Logging(#[from] PrePostError),
    #[error(transparent)]
    Segment(#[from] HttpError),
    #[error(transparent)]
    Store(#[from] VectorStoreError),
    #[error(transparent)]
    Url(#[from] UrlFailure),
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
    #[error("the request body could not be encoded")]
    Encode(#[source] serde_json::Error),
}
