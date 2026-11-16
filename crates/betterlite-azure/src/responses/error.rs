use betterlite_http::error::HttpError;
use betterlite_http::user_url::error::{InvalidUrl, UrlParseError};
use betterlite_openai::response_input::error::ResponseInputError;
use betterlite_openai::response_resources::error::ResponseResourceError;
use thiserror::Error;

use crate::azure_urls::AzureUrlError;

#[derive(Debug, Error)]
pub enum ResponsesError {
    #[error(transparent)]
    Url(#[from] AzureUrlError),
    #[error("api_base is required for Azure WebSocket")]
    MissingWebsocketApiBase,
    #[error("api_base is not a valid URL")]
    InvalidUrl(#[from] InvalidUrl),
    #[error("api_base does not parse as a URL")]
    Parse(#[from] UrlParseError),
    #[error("response_id cannot be a URL path segment")]
    ResponseId {
        #[source]
        source: HttpError,
    },
    // LiteLLM merges the nested mapping into the tool, so a non-object cannot be merged
    #[error("a tool's 'function' must be an object")]
    InvalidToolFunction,
    #[error(transparent)]
    Input(#[from] ResponseInputError),
    #[error(transparent)]
    Resource(#[from] ResponseResourceError),
    #[error("the provider response does not match the record: {0}")]
    Record(#[from] serde_json::Error),
}

impl ResponsesError {
    #[must_use]
    pub fn http_status(&self) -> Option<u16> {
        match self {
            Self::Resource(error) => error.http_status(),
            Self::Input(error) => error.http_status(),
            Self::Url(_)
            | Self::MissingWebsocketApiBase
            | Self::InvalidUrl(_)
            | Self::Parse(_)
            | Self::ResponseId { .. }
            | Self::InvalidToolFunction
            | Self::Record(_) => None,
        }
    }
}
