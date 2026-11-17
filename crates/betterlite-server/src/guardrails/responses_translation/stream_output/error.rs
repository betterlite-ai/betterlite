use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use thiserror::Error;

use crate::guardrails::responses_translation::output_rewrite::error::OutputRewriteError;
use crate::guardrails::responses_translation::stream_sync::error::StreamSyncError;

#[derive(Debug, Error)]
pub enum StreamOutputError {
    #[error(transparent)]
    Guardrail(#[from] GuardrailFailure),
    #[error(transparent)]
    Rewrite(#[from] OutputRewriteError),
    #[error(transparent)]
    Sync(#[from] StreamSyncError),
    #[error("the {target} could not be read as guardrail data")]
    Convert {
        target: &'static str,
        #[source]
        source: serde_json::Error,
    },
}
