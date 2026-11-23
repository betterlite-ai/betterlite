use betterlite_http::proxy::transport_error::TransportError;
use betterlite_http::user_url::error::InvalidUrl;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_openai::files::error::StatusError;
use http::header::{InvalidHeaderName, InvalidHeaderValue};
use thiserror::Error;

use crate::endpoints::initialize::EndpointError;

#[derive(Debug, Error)]
pub enum AzureSpeechError {
    #[error(transparent)]
    Client(#[from] EndpointError),
    #[error(
        "Must provide one of the `base_url` or `azure_endpoint` arguments, or the `AZURE_OPENAI_ENDPOINT` environment variable"
    )]
    MissingBase,
    #[error("the speech client base URL is not a URL")]
    Url {
        #[source]
        source: InvalidUrl,
    },
    #[error("the speech request URL `{url}` is not a URL")]
    RequestUrl {
        url: String,
        #[source]
        source: url::ParseError,
    },
    #[error("the {name} header name is not valid")]
    HeaderName {
        name: String,
        #[source]
        source: InvalidHeaderName,
    },
    #[error("the {name} header cannot carry its value")]
    HeaderValue {
        name: String,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the speech option `{option}` {problem}")]
    Option {
        option: String,
        problem: &'static str,
    },
    #[error("Unable to handle auth")]
    UnableToHandleAuth,
    #[error(
        "The api_key client option must be set either by passing api_key to the client or by setting the OPENAI_API_KEY environment variable"
    )]
    MissingKey,
    #[error(
        "Expected `azure_ad_token_provider` argument to return a string but it returned {returned}"
    )]
    TokenNotString { returned: String },
    #[error("{message}")]
    TokenProvider { class: String, message: String },
    #[error("the speech request could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error("the speech request could not be sent")]
    Transport {
        #[source]
        source: TransportError,
    },
    #[error("the speech logging hook failed")]
    Logging {
        #[source]
        source: PrePostError,
    },
    #[error("{}", .0.message)]
    Status(Box<StatusError>),
    #[error("{message}")]
    Handler {
        class: String,
        status: Option<u16>,
        message: String,
    },
    #[error("the speech client is not an Azure OpenAI or OpenAI client")]
    UnsupportedClient,
    #[error("model is not set")]
    ModelNotSet,
    #[error("invalid completion response")]
    InvalidCompletion,
    #[error("the speech call passes `{argument}` on its own and as an optional param")]
    DuplicateArgument { argument: &'static str },
    #[error("Unexpected JSON data type, {given}, cannot merge with `extra_body`")]
    ExtraBody { given: String },
}

impl AzureSpeechError {
    #[must_use]
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Client(error) => error.status(),
            Self::Status(error) => Some(error.status),
            Self::Handler { status, .. } => *status,
            _ => None,
        }
    }

    // the Python exception class each failure arrives as, for the oracle's error rows
    #[must_use]
    pub fn class(&self) -> &str {
        match self {
            Self::Status(_) => "APIStatusError",
            Self::UnableToHandleAuth => "ValueError",
            Self::MissingKey => "OpenAIError",
            Self::DuplicateArgument { .. } => "TypeError",
            Self::ExtraBody { .. } => "RuntimeError",
            Self::Client(error) => error.class(),
            Self::Handler { class, .. } => class,
            Self::MissingBase
            | Self::Url { .. }
            | Self::RequestUrl { .. }
            | Self::HeaderName { .. }
            | Self::HeaderValue { .. }
            | Self::Option { .. }
            | Self::TokenNotString { .. }
            | Self::TokenProvider { .. }
            | Self::Encode(_)
            | Self::Transport { .. }
            | Self::Logging { .. }
            | Self::UnsupportedClient
            | Self::ModelNotSet
            | Self::InvalidCompletion => "Exception",
        }
    }
}
