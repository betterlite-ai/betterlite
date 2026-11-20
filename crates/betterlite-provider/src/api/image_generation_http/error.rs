use std::error::Error;

use betterlite_openai::error::ProviderError;
use http::header::{InvalidHeaderName, InvalidHeaderValue};
use thiserror::Error;

// A provider config's own error keeps its type through the box, so its Display and source stay reachable
#[derive(Debug, Error)]
#[error(transparent)]
pub struct ConfigFailure {
    pub source: Box<dyn Error + Send + Sync>,
}

// The client source hands back a client of its own type, so its failure arrives the same way
#[derive(Debug, Error)]
#[error(transparent)]
pub struct ClientFailure {
    pub source: Box<dyn Error + Send + Sync>,
}

#[derive(Debug, Error)]
pub enum ImageGenerationError {
    #[error("{}", .0.message)]
    Provider(Box<ProviderError>),
    #[error(transparent)]
    Config(#[from] ConfigFailure),
    #[error(transparent)]
    Client(#[from] ClientFailure),
    #[error("{field} must be {expected}")]
    FieldType {
        field: &'static str,
        expected: &'static str,
    },
    #[error("optional_params.extra_headers must be an object of strings")]
    HeadersType,
    #[error("optional_params.extra_headers.{name} must be a string")]
    HeaderType { name: String },
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
    #[error("the {name} response header is not text")]
    HeaderText { name: String },
    #[error("the {field} form field must be a string, a number, a boolean, null or a list of them")]
    FormValue { field: String },
    #[error("the request body could not be encoded")]
    Encode(#[source] serde_json::Error),
}
