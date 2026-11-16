use betterlite_streaming::streamed_response_assembly::AssemblyError;
use thiserror::Error;

use crate::anthropic_sse::error::AnthropicSseError;
use crate::guardrail_base::failure::GuardrailFailure;

// What the response side of the tool permission guardrail can fail with
#[derive(Debug, Error)]
pub enum ResponseToolError {
    #[error(transparent)]
    Guardrail(#[from] GuardrailFailure),
    #[error(transparent)]
    Assembly(#[from] AssemblyError),
    #[error(transparent)]
    Sse(#[from] AnthropicSseError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
