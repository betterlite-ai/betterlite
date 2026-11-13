use betterlite_anthropic::anthropic_capability_catalog::thinking::ThinkingError;
use betterlite_anthropic::option_primitives::context_management::ContextManagementError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TransformError {
    #[error("max_tokens is required for Anthropic /v1/messages API")]
    MissingMaxTokens,
    // A request value the declared type cannot hold is a 400 that names the field.
    #[error("the field {field} must be an integer")]
    IntegerField { field: &'static str },
    // LiteLLM wraps a reasoning mapping failure in its own AnthropicError with status 400.
    #[error("{message}")]
    Anthropic { message: String },
    #[error(transparent)]
    Thinking(#[from] ThinkingError),
    #[error(transparent)]
    ContextManagement(#[from] ContextManagementError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl TransformError {
    #[must_use]
    pub const fn status_code(&self) -> u16 {
        match self {
            Self::MissingMaxTokens | Self::IntegerField { .. } | Self::Anthropic { .. } => 400,
            Self::Thinking(_) | Self::ContextManagement(_) | Self::Json(_) => 500,
        }
    }
}
