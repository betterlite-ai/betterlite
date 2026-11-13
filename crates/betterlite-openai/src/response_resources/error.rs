use betterlite_http::error::HttpError;
use thiserror::Error;

use crate::api::responses_shapes::wire::ShapeDecodeError;
use crate::error::OpenAiError;
use crate::response_input::error::ResponseInputError;

#[derive(Debug, Error)]
pub enum ResponseResourceError {
    #[error("response_id cannot be a URL path segment")]
    ResponseId {
        #[source]
        source: HttpError,
    },
    #[error("the API base is not a valid URL: {source}")]
    ApiBase {
        #[source]
        source: url::ParseError,
    },
    #[error(transparent)]
    Input(#[from] ResponseInputError),
    #[error("the provider response is not JSON: {source}")]
    Decode {
        #[source]
        source: ShapeDecodeError,
    },
    #[error("a decoded response does not serialize: {source}")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
    #[error("the provider response does not match the record: {source}")]
    Record {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Provider(#[from] OpenAiError),
}

impl ResponseResourceError {
    #[must_use]
    pub fn http_status(&self) -> Option<u16> {
        match self {
            Self::Provider(OpenAiError::Provider(provider)) => Some(provider.status_code),
            Self::Input(error) => error.http_status(),
            Self::Provider(_)
            | Self::ResponseId { .. }
            | Self::ApiBase { .. }
            | Self::Decode { .. }
            | Self::Encode { .. }
            | Self::Record { .. } => None,
        }
    }
}
