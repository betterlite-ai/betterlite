use std::convert::Infallible;
use std::error::Error;

use betterlite_models::resolution::ResolutionError;
use thiserror::Error;

use super::logging::LoggingUpdateError;
use super::params::RequestFieldError;

// E is the provider config's transform failure; the adapter path and dispatch never raise it.
#[derive(Debug, Error)]
pub enum GenAiSetupError<E = Infallible>
where
    E: Error + 'static,
{
    #[error("Mock response should be handled by caller")]
    MockResponseHandledByCaller,
    #[error("litellm_logging_obj is required, but got None")]
    LoggingRequired,
    #[error(transparent)]
    RequestField(#[from] RequestFieldError),
    #[error(transparent)]
    Resolution(#[from] ResolutionError),
    #[error(transparent)]
    Logging(#[from] LoggingUpdateError),
    #[error(transparent)]
    Transform(E),
}
