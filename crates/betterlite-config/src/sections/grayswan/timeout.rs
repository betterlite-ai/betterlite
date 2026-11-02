use std::time::{Duration, TryFromFloatSecsError};

use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Deserialize)]
#[serde(try_from = "f64")]
pub(super) struct GuardrailTimeout(pub(super) Duration);

#[derive(Debug, Error)]
#[error("guardrail_timeout must be a finite nonnegative duration")]
pub(super) struct GuardrailTimeoutError {
    #[source]
    source: TryFromFloatSecsError,
}

impl TryFrom<f64> for GuardrailTimeout {
    type Error = GuardrailTimeoutError;

    fn try_from(value: f64) -> Result<Self, Self::Error> {
        Duration::try_from_secs_f64(value)
            .map(Self)
            .map_err(|source| GuardrailTimeoutError { source })
    }
}
