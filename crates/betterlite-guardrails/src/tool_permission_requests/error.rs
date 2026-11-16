use thiserror::Error;

use crate::guardrail_base::failure::GuardrailFailure;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RequestToolsError {
    #[error("{0} must be {1}")]
    WrongType(String, &'static str),
}

impl From<RequestToolsError> for GuardrailFailure {
    fn from(error: RequestToolsError) -> Self {
        Self::Http {
            status: 400,
            detail: error.to_string(),
        }
    }
}
