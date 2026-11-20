use betterlite_logging::logging_pre_post::error::PrePostError;
use http::header::{InvalidHeaderName, InvalidHeaderValue};
use thiserror::Error;

use crate::api::vector_store_http_write::error::{
    BaseLlmException, ClientError, FailureFacts, handle_error,
};

use super::config::MetadataConfig;

#[derive(Debug, Error)]
pub enum MetadataError {
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
        source: url::ParseError,
    },
    #[error("the {name} query parameter is not text")]
    QueryValue { name: String },
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

// B2402's converter reduces the caught failure, then the provider config names the exception, as get_error_class does
#[must_use]
pub fn provider_error(facts: &FailureFacts, config: &dyn MetadataConfig) -> BaseLlmException {
    let failed = handle_error(facts, None);
    let synthesized = failed.status_code_is_synthesized;
    let mut raised = config.error_class(&failed.message, failed.status_code, failed.headers);
    raised.status_code_is_synthesized = synthesized;
    raised
}
