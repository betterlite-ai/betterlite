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
    #[error(transparent)]
    Transport(#[from] TransportError),
    #[error(transparent)]
    Logging(#[from] PrePostError),
    #[error("{}", .0.message)]
    Status(Box<StatusError>),
    #[error("the speech client base URL is not a URL")]
    BaseUrl(#[source] InvalidUrl),
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
    #[error("model is not set")]
    ModelNotSet,
    #[error("invalid completion response")]
    InvalidCompletion,
    #[error("the speech call passes `{argument}` on its own and as an optional param")]
    DuplicateArgument { argument: &'static str },
    #[error("Unexpected JSON data type, {given}, cannot merge with `extra_body`")]
    ExtraBody { given: &'static str },
    #[error("{message}")]
    Handler {
        class: String,
        status: Option<u16>,
        message: String,
    },
    #[error("the speech request could not be encoded")]
    Encode(#[source] serde_json::Error),
    #[error("the speech client is not an Azure OpenAI or OpenAI client")]
    UnsupportedClient,
}

impl AzureSpeechError {
    #[must_use]
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Client(error) => error.status(),
            Self::Status(error) => Some(error.status),
            Self::Handler { status, .. } => *status,
            Self::Transport(_)
            | Self::Logging(_)
            | Self::BaseUrl(_)
            | Self::RequestUrl { .. }
            | Self::HeaderName { .. }
            | Self::HeaderValue { .. }
            | Self::Option { .. }
            | Self::UnableToHandleAuth
            | Self::MissingKey
            | Self::ModelNotSet
            | Self::InvalidCompletion
            | Self::DuplicateArgument { .. }
            | Self::ExtraBody { .. }
            | Self::Encode(_)
            | Self::UnsupportedClient => None,
        }
    }

    // the Python exception class each failure arrives as
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
            Self::Transport(_)
            | Self::Logging(_)
            | Self::BaseUrl(_)
            | Self::RequestUrl { .. }
            | Self::HeaderName { .. }
            | Self::HeaderValue { .. }
            | Self::Option { .. }
            | Self::ModelNotSet
            | Self::InvalidCompletion
            | Self::Encode(_)
            | Self::UnsupportedClient => "Exception",
        }
    }
}
