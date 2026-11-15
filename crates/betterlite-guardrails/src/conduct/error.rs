use thiserror::Error;

use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_records::error::GuardrailRecordError;

#[derive(Debug, Error)]
pub enum ConductCheckError {
    #[error(
        "conduct-litellm-guard>=0.2.5 is required for the Conduct guardrail. Install it with: pip install \"conduct-litellm-guard>=0.2.5\""
    )]
    MissingPackage,
}

#[derive(Debug, Error)]
pub enum ConductError {
    #[error(transparent)]
    Blocked(#[from] GuardrailFailure),
    #[error("the Conduct check failed")]
    Check {
        #[source]
        source: ConductCheckError,
    },
    #[error(transparent)]
    Record(#[from] GuardrailRecordError),
}
