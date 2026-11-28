use std::error::Error;

use betterlite_anthropic::option_primitives::context_management::ContextManagementError;
use thiserror::Error;

// B2201 refuses a compact threshold whose int() truncation does not fit betterlite's bounded i64 JSON number
#[derive(Debug, Error)]
pub enum SpecError {
    #[error(transparent)]
    Threshold(#[from] ContextManagementError),
}

// an editor failure reaches the caller as the editor raised it: no translation and no best-effort recovery
#[derive(Debug, Error)]
pub enum DispatchError<Failure>
where
    Failure: Error + 'static,
{
    #[error(transparent)]
    Spec(#[from] SpecError),
    #[error(transparent)]
    Editor(Failure),
}
