use betterlite_prompts::arguments::json::Object;
use thiserror::Error;

use crate::generic_guardrail_contracts::UnreachableFallback;
use crate::typesafe::TypeSafeGuardrail;

pub const UNREACHABLE_STATUS_CODE: u16 = 502;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TypeSafeError {
    #[error("{message}")]
    Unavailable { message: String },
}

pub fn handle_failure(
    guardrail: &TypeSafeGuardrail,
    error: &str,
    log_detail: &Object,
) -> Result<(), TypeSafeError> {
    match guardrail.settings.unreachable_fallback {
        UnreachableFallback::FailOpen => {
            tracing::warn!(
                detail = ?log_detail,
                "TypeSafe: {error}; fail_open configured, forwarding request uncompacted"
            );
            Ok(())
        }
        UnreachableFallback::FailClosed => {
            tracing::error!(detail = ?log_detail, "TypeSafe: {error}");
            Err(TypeSafeError::Unavailable {
                message: error.to_owned(),
            })
        }
    }
}

#[path = "typesafe_failure_handling/tests.rs"]
mod tests;
