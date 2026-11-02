use thiserror::Error;

// Mirrors LiteLLM's CiscoAIDefenseGuardrailAPIError, raised for every transport failure.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("{0}")]
pub struct ApiError(pub String);

mod tests;
