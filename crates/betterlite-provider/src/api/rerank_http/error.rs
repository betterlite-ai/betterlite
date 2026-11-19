use std::error::Error;

use betterlite_config::tls::TlsPolicyError;
use betterlite_http::proxy::pool::PoolError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use http::header::{InvalidHeaderName, InvalidHeaderValue};
use thiserror::Error;
use url::ParseError;

use crate::api::rerank_contracts::BaseLlmException;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error(transparent)]
    Tls(#[from] TlsPolicyError),
    #[error(transparent)]
    Pool(#[from] PoolError),
}

// The provider error is the one a working setup catches; the rest name a failure that never reached the provider
#[derive(Debug, Error)]
pub enum RerankHttpError<Failure>
where
    Failure: Error + Send + Sync + 'static,
{
    #[error(transparent)]
    Config(#[source] Failure),
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
