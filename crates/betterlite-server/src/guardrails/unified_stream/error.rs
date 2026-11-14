use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::unified_dispatch::emit::StreamHttpError;
use thiserror::Error;

// The failures that leave the loop instead of becoming a stream message
#[derive(Debug, PartialEq, Error)]
pub enum StreamLoopError {
    #[error("the guardrail failed while transforming the streamed response")]
    Guardrail {
        #[source]
        source: GuardrailFailure,
    },
    #[error("the guardrail blocked the streamed response and no block message could be written")]
    Block {
        #[source]
        source: GuardrailFailure,
    },
    #[error("the streamed error could not be written into the response stream")]
    Http {
        #[source]
        source: StreamHttpError,
    },
}

// The terminal chunks of a block or an in-stream error were already queued when this is raised
#[derive(Debug, Error)]
#[error("the incremental transform stream already wrote its terminal chunks")]
pub(crate) struct StreamTerminated;

#[derive(Debug, Error)]
pub(crate) enum RoundFailure {
    #[error(transparent)]
    Terminated(#[from] StreamTerminated),
    #[error(transparent)]
    Loop(#[from] StreamLoopError),
}
