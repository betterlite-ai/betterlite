use crate::builtin::PromptError;
use crate::jinja::RenderError;
use betterlite_logging::analytics::python::FormatError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FactoryError {
    #[error(transparent)]
    Prompt(#[from] PromptError),
    #[error(transparent)]
    Render(#[from] RenderError),
    #[error("content cannot be formatted")]
    Repr(#[from] FormatError),
    #[error("prompt exceeds its byte budget")]
    Limit,
    #[error("input exceeds its byte budget")]
    InputLimit(#[source] serde_json::Error),
    #[error("message {index} content must be text or a list to receive the function prompt")]
    Append { index: usize },
    #[error("No chat template found")]
    NoChatTemplate,
    #[error("betterlite does not support the {operation} prompt adapter yet")]
    Unsupported { operation: &'static str },
}
