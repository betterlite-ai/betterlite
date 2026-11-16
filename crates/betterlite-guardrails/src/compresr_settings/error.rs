use std::num::TryFromIntError;
use std::time::TryFromFloatSecsError;

use thiserror::Error;

use crate::compresr_transport::error::CompresrTransportError;

#[derive(Debug, Error)]
pub enum CompresrSettingsError {
    #[error(
        "Compresr guardrail requires an API key. Set `api_key` in the guardrail config or the COMPRESR_API_KEY env var."
    )]
    MissingApiKey,
    #[error("max_bytes_per_call must be >= 0 (0 disables the cap; positive values enforce it)")]
    ByteCap {
        #[source]
        source: TryFromIntError,
    },
    #[error("the Compresr timeout of {seconds} seconds is not a valid duration")]
    Timeout {
        seconds: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error(transparent)]
    Transport(#[from] CompresrTransportError),
}
