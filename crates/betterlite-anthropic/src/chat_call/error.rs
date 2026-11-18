use std::error::Error as StdError;

use betterlite_error::error::ErrorClass;
use betterlite_error::error::metadata::ErrorMetadata;
use betterlite_logging::logging_pre_post::error::PrePostError;
use http::HeaderMap;
use http::header::{InvalidHeaderName, InvalidHeaderValue};
use thiserror::Error;

use crate::anthropic_http_stream::completion::{CompletionError, ParameterError};
use crate::anthropic_http_stream::error::StreamOpenError;
use crate::headers::HeadersError;
use crate::response::error::ResponseError;

// The selected provider config's own failures; the config decides their type, this leaf only passes them on
pub type ConfigError = Box<dyn StdError + Send + Sync>;
// A config refusal that raised a typed LiteLLM error keeps its class, so the call projects the same status and fields
#[derive(Debug, Error)]
#[error("{source}")]
pub struct ClassifiedConfigError {
    #[source]
    pub source: Box<dyn StdError + Send + Sync>,
    pub class: ErrorClass,
}

// Only a config that raised a typed error names a class; a plain failure has none, like the bare exception LiteLLM re-raises
fn config_metadata(source: &ConfigError) -> Option<ErrorMetadata> {
    source
        .downcast_ref::<ClassifiedConfigError>()
        .map(|held| held.class.metadata())
}
#[derive(Debug, Error)]
pub enum ChatCallError<ClientFailure, CounterFailure>
where
    ClientFailure: StdError + Send + Sync + 'static,
    CounterFailure: StdError + Send + Sync + 'static,
{
    #[error(transparent)]
    Headers(#[from] HeadersError),
    #[error("the provider config could not prepare the request: {source}")]
    Prepare {
        #[source]
        source: ConfigError,
    },
    #[error("the provider config could not filter the request betas: {source}")]
    Filter {
        #[source]
        source: ConfigError,
    },
    #[error(transparent)]
    PreCall(#[from] PrePostError),
    #[error(transparent)]
    Response(#[from] ResponseError<CounterFailure>),
    #[error(transparent)]
    Parameter(#[from] ParameterError),
    #[error(transparent)]
    Open(#[from] StreamOpenError<ClientFailure>),
    #[error("{message}")]
    Anthropic {
        status_code: u16,
        message: String,
        headers: Option<HeaderMap>,
    },
    #[error("the api_base is not a URL")]
    Endpoint(#[from] url::ParseError),
    #[error("a request header name is not valid")]
    HeaderName(#[from] InvalidHeaderName),
    #[error("a request header value is not valid")]
    HeaderValue(#[from] InvalidHeaderValue),
}
impl<ClientFailure, CounterFailure> ChatCallError<ClientFailure, CounterFailure>
where
    ClientFailure: StdError + Send + Sync + 'static,
    CounterFailure: StdError + Send + Sync + 'static,
{
    // The fields a config refusal carries into the response, when the config raised a typed LiteLLM error
    #[must_use]
    pub fn metadata(&self) -> Option<ErrorMetadata> {
        match self {
            Self::Prepare { source } | Self::Filter { source } => config_metadata(source),
            Self::Headers(_)
            | Self::PreCall(_)
            | Self::Response(_)
            | Self::Parameter(_)
            | Self::Open(_)
            | Self::Anthropic { .. }
            | Self::Endpoint(_)
            | Self::HeaderName(_)
            | Self::HeaderValue(_) => None,
        }
    }
    // The status LiteLLM puts on the exception it raises from this call, as far as the error carries one
    #[must_use]
    pub fn status_code(&self) -> Option<u16> {
        match self {
            Self::Headers(source) => source.status_code(),
            Self::Prepare { source } | Self::Filter { source } => {
                config_metadata(source).map(|metadata| metadata.status_code)
            }
            Self::PreCall(_) => None,
            Self::Response(source) => match source {
                ResponseError::Anthropic { status, .. } => Some(*status),
                ResponseError::Decode(_)
                | ResponseError::MissingField(_)
                | ResponseError::MissingChoice
                | ResponseError::FieldType { .. }
                | ResponseError::Parameter { .. }
                | ResponseError::Content(_)
                | ResponseError::Usage { .. } => None,
            },
            Self::Parameter(_) => Some(400),
            Self::Open(source) => match source {
                StreamOpenError::Anthropic { status_code, .. } => Some(*status_code),
                // make_call reports an unknown send failure as AnthropicError(status_code=500)
                StreamOpenError::Unknown { .. } | StreamOpenError::Endpoint { .. } => Some(500),
                // a re-raised litellm exception keeps its own class and status, which this variant cannot see
                StreamOpenError::Reraised { .. } => None,
            },
            Self::Anthropic { status_code, .. } => Some(*status_code),
            // httpx raises while building or sending, and the handler catches every one of those as a 500
            Self::Endpoint(_) | Self::HeaderName(_) | Self::HeaderValue(_) => Some(500),
        }
    }
}
// The stream helper reports the two failures this call already wraps, so ? converts them without a mapping step
impl<ClientFailure, CounterFailure> From<CompletionError<ClientFailure>>
    for ChatCallError<ClientFailure, CounterFailure>
where
    ClientFailure: StdError + Send + Sync + 'static,
    CounterFailure: StdError + Send + Sync + 'static,
{
    fn from(source: CompletionError<ClientFailure>) -> Self {
        match source {
            CompletionError::Parameter(error) => Self::Parameter(error),
            CompletionError::Open(error) => Self::Open(error),
        }
    }
}
