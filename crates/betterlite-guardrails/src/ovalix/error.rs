use thiserror::Error;

use super::exception_status::{GuardrailRaisedException, OvalixBlockedException};

#[derive(Debug, Error)]
pub enum OvalixError {
    #[error(transparent)]
    Blocked(#[from] OvalixBlockedException),
    #[error(transparent)]
    Raised(#[from] GuardrailRaisedException),
}
