use betterlite_anthropic::tools::ToolsError;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_message_scope::unappliable_rewrite::UnappliableRequestRewrite;
use thiserror::Error;

use crate::guardrails::anthropic_translation::input_scan::error::ConvertError;
use crate::guardrails::anthropic_translation::structured_write_back::error::StructuredWriteBackError;
use crate::guardrails::anthropic_translation::write_back::error::{ApplyError, WriteBackError};

#[derive(Debug, Error)]
pub enum ProcessInputError {
    #[error("the Anthropic guardrail failed the request")]
    Guardrail {
        #[source]
        source: Box<GuardrailFailure>,
    },
    #[error("the guardrail's rewrite cannot be applied to this request")]
    Unappliable(#[from] UnappliableRequestRewrite),
    #[error("the guardrail's structured messages cannot be written back")]
    Structured(#[from] StructuredWriteBackError),
    #[error("the guardrail's rewritten texts cannot be written back")]
    Text(#[from] WriteBackError),
    #[error("the guardrail's rewritten tool calls cannot be written back")]
    ToolCalls(#[from] ApplyError),
    #[error("a returned tool could not be turned into an Anthropic tool")]
    Tool(#[from] ToolsError),
    #[error(transparent)]
    Convert(#[from] ConvertError),
    #[error("the request field '{field}' must be {expected}")]
    FieldType {
        field: &'static str,
        expected: &'static str,
    },
}
