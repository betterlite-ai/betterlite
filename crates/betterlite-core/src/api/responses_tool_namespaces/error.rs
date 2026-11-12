use thiserror::Error;

use crate::api::responses_codex_tools::error::CustomToolsError;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum NamespaceToolsError {
    #[error("Top-level function names conflict with flattened namespace tools: {names}")]
    Conflict { names: String },
    #[error("tool {position} must be an object")]
    ToolNotObject { position: usize },
    #[error("{field} must be {expected}")]
    WrongType {
        field: &'static str,
        expected: &'static str,
    },
    #[error(transparent)]
    Custom(#[from] CustomToolsError),
}
