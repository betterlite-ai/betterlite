use crate::generic_guardrail_contracts::UnreachableFallback;

use super::analysis::Source;
use super::error::VigilGuardError;
use super::output::{Inputs, ToolCall, build_output};

pub struct FailureContext<'guardrail> {
    pub fallback: UnreachableFallback,
    pub guardrail_name: &'guardrail str,
    pub source: Source,
}

pub fn handle_backend_failure(
    context: &FailureContext<'_>,
    error: &VigilGuardError,
    inputs: &Inputs,
    final_texts: &[String],
    final_tool_calls: Option<&[ToolCall]>,
) -> Result<Inputs, VigilGuardError> {
    match context.fallback {
        UnreachableFallback::FailOpen => {
            tracing::error!(
                guardrail_name = context.guardrail_name,
                source = ?context.source,
                error = %error,
                "Vigil Guard backend failure with fail_open; allowing request unscanned"
            );
            Ok(build_output(inputs, final_texts, final_tool_calls))
        }
        UnreachableFallback::FailClosed => {
            tracing::error!(
                guardrail_name = context.guardrail_name,
                source = ?context.source,
                error = %error,
                "Vigil Guard backend failure with fail_closed; blocking request"
            );
            Err(VigilGuardError::Blocked {
                reason: "the backend is unreachable; the fail_closed policy blocked it".to_owned(),
            })
        }
    }
}
