use betterlite_core::api::responses_tool_namespaces::error::NamespaceToolsError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ToolMergeError {
    #[error(transparent)]
    Tools(#[from] NamespaceToolsError),
}
