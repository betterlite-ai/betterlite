use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::io;

use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use betterlite_openai::error::ProviderError;
use thiserror::Error;

// The header map B2402's conversion hands get_error_class: a name to a JSON value, as LiteLLM passes dict(error_headers)
pub type ErrorHeaders = BTreeMap<String, Json>;

// A config or client failure crosses the trait boundary boxed, as B2402's UrlFailure does
pub type TransportSource = Box<dyn StdError + Send + Sync + 'static>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display)]
#[strum(serialize_all = "lowercase")]
pub enum Operation {
    Remix,
    List,
}

// The provider config members these handlers call fail with their own error, which travels boxed
#[derive(Debug, Error)]
#[error(transparent)]
pub struct ConfigFailure {
    pub source: TransportSource,
}

#[derive(Debug, Error)]
pub enum FieldError {
    #[error("litellm_params.{field} must be {expected}")]
    Type {
        field: &'static str,
        expected: &'static str,
    },
}

// Everything a handler can fail with: a failure inside the try block is a Provider error (B2402's conversion), and every other variant is raised outside it, where LiteLLM raises before _handle_error
#[derive(Debug, Error)]
pub enum VideoRemixListTransportError {
    #[error("{}", .0.message)]
    Provider(Box<ProviderError>),
    #[error(transparent)]
    Field(#[from] FieldError),
    #[error("\"{provider}\" is not a provider betterlite knows")]
    UnknownProvider { provider: String },
    #[error("the video HTTP client could not be built")]
    Client {
        #[source]
        source: TransportSource,
    },
    #[error("the video {operation} request could not be prepared")]
    Prepare {
        operation: Operation,
        #[source]
        source: ConfigFailure,
    },
    #[error(transparent)]
    Logging(#[from] PrePostError),
    #[error("video list sync handler cannot run inside a running runtime")]
    RunningRuntime { status_code: u16 },
    #[error("the video list sync runtime could not be started")]
    Runtime {
        #[source]
        source: io::Error,
    },
}

impl From<ProviderError> for VideoRemixListTransportError {
    fn from(error: ProviderError) -> Self {
        Self::Provider(Box::new(error))
    }
}
