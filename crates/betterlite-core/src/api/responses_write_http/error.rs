use std::str::Utf8Error;

use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::response_resources::error::ResponseResourceError;
use http::header::{InvalidHeaderName, InvalidHeaderValue};
use thiserror::Error;

use crate::api::vector_store_http_write::error::{
    BaseLlmException, ClientError, FailureFacts, UrlFailure, handle_error as base_handle_error,
};

use super::config::ResponsesConfig;

#[derive(Debug, Error)]
pub enum WriteError {
    #[error(transparent)]
    Provider(#[from] BaseLlmException),
    #[error(transparent)]
    Client(#[from] ClientError),
    #[error(transparent)]
    Logging(#[from] PrePostError),
    #[error(transparent)]
    Resource(#[from] ResponseResourceError),
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
    #[error("the reply body is not UTF-8")]
    ReplyText(#[source] Utf8Error),
}

// B2402's _handle_error builds the status, text and headers; the provider's error class is the responses hook
#[must_use]
pub fn handle_error(facts: &FailureFacts, config: &dyn ResponsesConfig) -> BaseLlmException {
    let converted = base_handle_error(facts, None);
    let synthesized = converted.status_code_is_synthesized;
    let mut error =
        config.error_class(&converted.message, converted.status_code, converted.headers);
    error.status_code_is_synthesized = synthesized;
    error
}
