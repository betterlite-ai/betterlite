use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use thiserror::Error;

use crate::guardrails::anthropic_translation::input_scan::error::ConvertError;

#[derive(Debug, Error)]
pub enum ProcessOutputError {
    #[error("the Anthropic guardrail failed the response")]
    Guardrail {
        #[source]
        source: Box<GuardrailFailure>,
    },
    #[error("the guardrail returned {returned} texts for {sent} scanned texts")]
    TooManyTexts { sent: usize, returned: usize },
    #[error("the tool_use block at content position {content_idx} is missing id, name or input")]
    MalformedToolUse { content_idx: usize },
    #[error(transparent)]
    Convert(#[from] ConvertError),
}
