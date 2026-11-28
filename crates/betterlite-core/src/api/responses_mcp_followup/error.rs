use std::convert::Infallible;

use thiserror::Error;

use crate::api::responses_mcp_events::error::McpEventError;
use crate::api::responses_mcp_tool_selection::ToolSelectionError;

use super::streaming::StreamingError;

#[derive(Debug, Error)]
pub enum ToolExecutionEventError {
    #[error(transparent)]
    Details(#[from] ToolSelectionError),
    #[error("tool call arguments must be a string to stream as MCP call arguments")]
    ArgumentsNotString,
    #[error(transparent)]
    Events(#[from] McpEventError),
}

#[derive(Debug, Error)]
pub enum McpFollowupError<ProviderError = Infallible> {
    #[error(transparent)]
    Events(#[from] ToolExecutionEventError),
    #[error(transparent)]
    Streaming(#[from] StreamingError),
    #[error("the follow-up responses call failed")]
    Provider(#[source] ProviderError),
}
