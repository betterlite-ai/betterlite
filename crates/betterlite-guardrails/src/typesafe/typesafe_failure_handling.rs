use betterlite_prompts::arguments::json::Object;
use reqwest::StatusCode;
use thiserror::Error;

use crate::generic_guardrail_contracts::UnreachableFallback;
use crate::typesafe::TypeSafeGuardrail;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TypeSafeError {
    #[error("{message}")]
    Unavailable { message: String },
}

impl TypeSafeError {
    // LiteLLM raises HTTPException(status_code=502, detail={"error": message}) on the fail_closed path.
    #[must_use]
    pub const fn status_code(&self) -> StatusCode {
        match self {
            Self::Unavailable { .. } => StatusCode::BAD_GATEWAY,
        }
    }
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
