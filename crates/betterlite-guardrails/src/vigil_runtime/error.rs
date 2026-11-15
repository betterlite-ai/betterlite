use std::time::TryFromFloatSecsError;

use thiserror::Error;

use crate::guardrail_base::error::GuardrailBaseError;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::vigil_guard::error::VigilGuardError;

#[derive(Debug, Error)]
pub enum VigilRuntimeError {
    #[error(transparent)]
    Guard(#[from] VigilGuardError),
    #[error(transparent)]
    Base(#[from] GuardrailBaseError),
    #[error("the Vigil Guard timeout of {seconds} seconds is not a valid duration")]
    Timeout {
        seconds: f64,
        #[source]
        source: TryFromFloatSecsError,
    },
    #[error("the Vigil Guard tool_calls entry at index {index} is not an object")]
    InvalidToolCall { index: usize },
}

impl From<VigilRuntimeError> for GuardrailFailure {
    fn from(error: VigilRuntimeError) -> Self {
        Self::Other(error.to_string())
    }
}

impl From<VigilGuardError> for GuardrailFailure {
    fn from(error: VigilGuardError) -> Self {
        Self::Other(error.to_string())
    }
}
