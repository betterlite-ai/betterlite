use betterlite_guardrails::guardrail_selection::error::GuardrailSelectionError;
use thiserror::Error;

use super::settings::InvalidFlag;
use crate::guardrails::unified_stream::error::StreamLoopError;

#[derive(Debug, PartialEq, Error)]
pub enum StreamHookError<E> {
    #[error(transparent)]
    InvalidFlag(#[from] InvalidFlag),
    #[error("the post-call gate of the guardrail failed")]
    Gate {
        #[source]
        source: GuardrailSelectionError,
    },
    #[error(transparent)]
    Loop(#[from] StreamLoopError<E>),
}
