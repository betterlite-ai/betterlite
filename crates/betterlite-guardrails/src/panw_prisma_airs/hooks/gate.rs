use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use super::context::HookContext;
use super::data::Data;
use crate::guardrail_selection::error::GuardrailSelectionError;

// LiteLLM's PanwPrismaAirsHandler.should_run_guardrail: an MCP event also asks the base hook it stands in for.
pub fn should_run_guardrail(
    data: &Data,
    event: GuardrailEventHooks,
    context: &HookContext<'_>,
) -> Result<bool, GuardrailSelectionError> {
    if context
        .selection
        .should_run_guardrail(data, event, context.suppressed)?
    {
        return Ok(true);
    }
    let Some(compat) = compatible_event(event) else {
        return Ok(false);
    };
    context
        .selection
        .should_run_guardrail(data, compat, context.suppressed)
}

// LiteLLM's _MCP_COMPAT_MAP.
#[must_use]
pub const fn compatible_event(event: GuardrailEventHooks) -> Option<GuardrailEventHooks> {
    match event {
        GuardrailEventHooks::PreMcpCall => Some(GuardrailEventHooks::PreCall),
        GuardrailEventHooks::DuringMcpCall => Some(GuardrailEventHooks::DuringCall),
        GuardrailEventHooks::PreCall
        | GuardrailEventHooks::PostCall
        | GuardrailEventHooks::DuringCall
        | GuardrailEventHooks::LoggingOnly
        | GuardrailEventHooks::PostMcpCall
        | GuardrailEventHooks::RealtimeInputTranscription => None,
    }
}
