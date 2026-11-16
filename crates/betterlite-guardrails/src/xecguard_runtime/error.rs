use std::time::{SystemTimeError, TryFromFloatSecsError};

use thiserror::Error;

use crate::presidio_response::error::PresidioResponseError;
use crate::xecguard_primitives::error::XecGuardError;

#[derive(Debug, Error)]
pub enum XecGuardRuntimeError {
    #[error(
        "XecGuard API key is required. Set XECGUARD_API_KEY in the environment or pass api_key in the guardrail config."
    )]
    MissingApiKey,
    #[error("the XecGuard timeout is not a representable duration")]
    UnrepresentableTimeout {
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error(transparent)]
    Api(#[from] XecGuardError),
    #[error("standard_logging_object is not an object")]
    StandardLoggingObject,
    #[error("the logged values are not valid JSON")]
    LoggedJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("the logged result cannot be written as JSON")]
    ResultBody {
        #[source]
        source: PresidioResponseError,
    },
    #[error("the system clock is before the Unix epoch")]
    Clock {
        #[source]
        source: SystemTimeError,
    },
}
