use thiserror::Error;

use crate::api::responses_tool_namespaces::error::NamespaceToolsError;
use crate::api::responses_usage::ResponsesUsageError;

#[derive(Debug, Error)]
pub enum ChatOutputError {
    #[error("the chat response field {field} could not be read: {source}")]
    Json {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the chat response field {field} must be {expected}")]
    WrongType {
        field: &'static str,
        expected: &'static str,
    },
    #[error("the chat response field {field} is not a number: {source}")]
    Number {
        field: &'static str,
        #[source]
        source: rust_decimal::Error,
    },
    #[error("the chat response usage could not be read: {source}")]
    Usage {
        #[source]
        source: ResponsesUsageError,
    },
    #[error("the request tool choice could not be read: {source}")]
    ToolChoice {
        #[source]
        source: serde_json::Error,
    },
    #[error("the assembled response could not be built: {source}")]
    Response {
        #[source]
        source: serde_json::Error,
    },
    #[error(transparent)]
    Tools(#[from] NamespaceToolsError),
}
