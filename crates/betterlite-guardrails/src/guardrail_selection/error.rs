use thiserror::Error;

use crate::guardrail_base::failure::GuardrailFailure;

// LiteLLM prints a missing guardrail name as None
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GuardrailSelectionError {
    #[error(
        "Guardrail {} uses a tag-based event hook, which betterlite does not support",
        .guardrail_name.as_deref().unwrap_or("None")
    )]
    TagMode { guardrail_name: Option<String> },
    #[error("The requested guardrails must be a list")]
    RequestedNotList,
    #[error("The request options for guardrail {guardrail_name} must be an object")]
    DynamicParamsNotObject { guardrail_name: String },
}

impl From<GuardrailSelectionError> for GuardrailFailure {
    fn from(error: GuardrailSelectionError) -> Self {
        let status = match error {
            GuardrailSelectionError::TagMode { .. } => 500,
            GuardrailSelectionError::RequestedNotList
            | GuardrailSelectionError::DynamicParamsNotObject { .. } => 400,
        };
        Self::Http {
            status,
            detail: error.to_string(),
        }
    }
}
