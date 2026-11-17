use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use betterlite_openai::files::stream::flatten_headers;
use http::HeaderMap;
use thiserror::Error;

use super::decode::DecodeError;
use super::{CREATE_NOT_IMPLEMENTED, MISSING_API_KEY, MISSING_API_KEY_VALIDATOR};

#[derive(Debug, Error)]
pub enum BatchesError {
    #[error("{}", CREATE_NOT_IMPLEMENTED)]
    NotImplemented,
    #[error("{}", MISSING_API_KEY)]
    MissingApiKey,
    #[error("{}", MISSING_API_KEY_VALIDATOR)]
    MissingEnvironmentKey,
    #[error("the {name} header value is not a valid HTTP header value")]
    InvalidHeader { name: String },
    #[error("the api_base is not a URL")]
    InvalidBase {
        #[source]
        source: url::ParseError,
    },
    #[error(transparent)]
    Segment(#[from] HttpError),
    #[error(transparent)]
    Decode(#[from] DecodeError),
    #[error("the request could not be sent")]
    Transport {
        #[source]
        source: TransportError,
    },
    #[error("the batch call could not be logged")]
    Logging {
        #[source]
        source: PrePostError,
    },
    #[error("{}", .0.message)]
    Provider(Box<ProviderError>),
}

impl BatchesError {
    #[must_use]
    pub fn provider(message: &str, status: u16, headers: Option<&HeaderMap>) -> Self {
        let flattened = headers.map(|values| flatten_headers(values).into_iter().collect());
        Self::Provider(Box::new(ProviderError::from(ProviderErrorParts {
            status_code: status,
            message: message.to_owned(),
            headers: flattened,
            request: None,
            response: None,
            body: None,
            status_code_is_synthesized: false,
        })))
    }
}
