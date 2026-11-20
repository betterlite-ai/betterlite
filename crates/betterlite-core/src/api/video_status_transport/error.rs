use betterlite_http::proxy::pool::PoolError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use thiserror::Error;

use super::config::VideoErrorClasses;
use super::records::Headers;

// LiteLLM's BaseLLMException: the provider's own error class, with the status and headers it carries
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct BaseLlmException {
    pub status_code: u16,
    pub message: String,
    pub headers: Headers,
    pub status_code_is_synthesized: bool,
}

// A provider transform's failure: str(e), plus the status and headers the exception carries when it has them
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct ProviderFailure {
    pub message: String,
    pub status_code: Option<u16>,
    pub headers: Option<Headers>,
}

// The caught exception the shared converter reads
#[derive(Debug, Clone, Copy)]
pub enum StatusFailure<'failure> {
    Send(&'failure TransportError),
    Transform(&'failure ProviderFailure),
}

// B2402's shared _handle_error: it reads the failure's facts and asks the provider config for its error class
pub trait ErrorConverter: Send + Sync {
    fn convert(
        &self,
        config: &dyn VideoErrorClasses,
        failure: StatusFailure<'_>,
    ) -> BaseLlmException;
}

#[derive(Debug, Error)]
pub enum VideoStatusError {
    #[error(transparent)]
    Converted(#[from] BaseLlmException),
    #[error(transparent)]
    Unconverted(#[from] ProviderFailure),
    #[error(transparent)]
    Client(#[from] PoolError),
    #[error(transparent)]
    Logging(#[from] PrePostError),
    #[error("unknown video provider {provider}")]
    UnknownProvider { provider: String },
    #[error("the {field} parameter must be {expected}")]
    FieldType {
        field: &'static str,
        expected: &'static str,
    },
    #[error("the blocking status call waited on a client that never answered")]
    Suspended,
}
