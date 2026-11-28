use std::error::Error;

use betterlite_core::api::vector_store_http_write::error::ClientError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use http::header::{InvalidHeaderName, InvalidHeaderValue};
use thiserror::Error;
use url::ParseError;

use crate::api::rerank_contracts::BaseLlmException;

// The provider error is the one a working setup catches; the rest name a failure that never reached the provider
#[derive(Debug, Error)]
pub enum RerankHttpError<Failure>
where
    Failure: Error + Send + Sync + 'static,
{
    #[error(transparent)]
    Config(Failure),
    #[error(transparent)]
    Provider(#[from] BaseLlmException),
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error(transparent)]
    Logging(#[from] PrePostError),
    #[error("the request URL {url} is not a URL")]
    RequestUrl {
        url: String,
        #[source]
        source: ParseError,
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
