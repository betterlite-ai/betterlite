use thiserror::Error;

use crate::guardrail_base::error::GuardrailBaseError;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ToolPolicyError {
    #[error(transparent)]
    Base(#[from] GuardrailBaseError),
}
