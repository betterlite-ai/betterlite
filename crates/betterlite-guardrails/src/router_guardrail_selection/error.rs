use thiserror::Error;

#[derive(Debug, Error)]
pub enum GuardrailSelectionError {
    #[error("No guardrail found with name: {guardrail_name}")]
    NotFound { guardrail_name: String },
}
