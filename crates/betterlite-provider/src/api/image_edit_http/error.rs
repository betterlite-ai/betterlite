use std::error::Error;

use betterlite_core::api::vector_store_http_write::error::ClientError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::error::ProviderError;
use http::header::{InvalidHeaderName, InvalidHeaderValue};
use thiserror::Error;

// The request build's failures: a typed litellm_params value of the wrong type, an unrepresentable form value, or an invalid URL or header
#[derive(Debug, Error)]
pub enum RequestError {
    #[error("the {field} value has the wrong type")]
    FieldType { field: String },
    #[error("the {field} value cannot be sent as a multipart field")]
    FormValue { field: String },
    #[error("the resolved request URL {url} is not a URL")]
    Url {
        url: String,
        #[source]
        source: url::ParseError,
    },
    #[error("the {name} header cannot be a request header name")]
    HeaderName {
        name: String,
        #[source]
        source: InvalidHeaderName,
    },
    #[error("the {name} header's value cannot be carried")]
    HeaderValue {
        name: String,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the finalized request data could not be written as JSON")]
    Encode(#[source] serde_json::Error),
}

// The handler's failures: the provider's answer through the config's error hook, the config's own refusals, and the request build
#[derive(Debug, Error)]
pub enum ImageEditHttpError<ConfigError: Error + 'static> {
    #[error("{}", .0.message)]
    Provider(Box<ProviderError>),
    #[error("the image edit provider config failed: {source}")]
    Config(#[source] ConfigError),
    #[error(transparent)]
    Request(#[from] RequestError),
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error(transparent)]
    Logging(#[from] PrePostError),
}
