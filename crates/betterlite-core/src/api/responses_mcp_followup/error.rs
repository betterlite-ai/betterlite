use std::convert::Infallible;

use thiserror::Error;

use crate::api::responses_mcp_events::error::McpEventError;
use crate::api::responses_mcp_tool_selection::ToolSelectionError;

use super::streaming::StreamingError;

// every failure _create_tool_execution_events (L1224-L1272) can report: a tool call B10122 refuses to read, non-text arguments, or the builder's overflow check
#[derive(Debug, Error)]
pub enum ToolExecutionEventError {
    #[error(transparent)]
    Details(#[from] ToolSelectionError),
    #[error("tool call arguments must be a string to stream as MCP call arguments")]
    ArgumentsNotString,
    #[error(transparent)]
    Events(#[from] McpEventError),
}

// one error type for the whole follow-up stage, widened as the provider's error joins it
#[derive(Debug, Error)]
pub enum McpFollowupError<ProviderError = Infallible> {
    #[error(transparent)]
    Events(#[from] ToolExecutionEventError),
    #[error(transparent)]
    Streaming(#[from] StreamingError),
    #[error("the follow-up responses call failed")]
    Provider(#[source] ProviderError),
}
