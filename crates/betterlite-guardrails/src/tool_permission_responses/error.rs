use betterlite_streaming::streamed_response_assembly::AssemblyError;
use thiserror::Error;

use crate::guardrail_base::failure::GuardrailFailure;

// What the response side of the tool permission guardrail can fail with
#[derive(Debug, Error)]
pub enum ResponseToolError {
    #[error(transparent)]
    Guardrail(#[from] GuardrailFailure),
    #[error(transparent)]
    Assembly(#[from] AssemblyError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    // LiteLLM's parse_tool_call_arguments ValueError, re-raised while an Anthropic response is re-encoded
    #[error(
        "Failed to parse tool call arguments for tool '{tool}' ({context}). Error: {message}. Arguments: {arguments}"
    )]
    ToolArguments {
        tool: String,
        context: String,
        message: String,
        arguments: String,
    },
}
