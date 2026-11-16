use std::collections::BTreeSet;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_message_scope::Object;
use crate::guardrail_selection::gate::GuardrailSelection as _;
use crate::tool_permission::ToolPermissionGuardrail;
use crate::tool_permission_requests::early_return::guardrail_base;
use crate::tool_permission_requests::header_mutation::record_applied_guardrail;

use super::anthropic_conversion::{anthropic_content_blocks, extract_anthropic_tools};
use super::anthropic_rewrite::rewrite_anthropic_responses;
use super::evaluate::evaluate_tool_calls;
use super::extraction::extract_response_tools;
use super::openai_rewrite::rewrite_openai_responses;

// Which response body the caller handed the hook, LiteLLM's isinstance(response, ModelResponse) and its dict fallback
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseFormat {
    OpenAi,
    Anthropic,
}

// LiteLLM's async_post_call_success_hook: the complete response is checked and rewritten in place
pub fn post_call_success_hook(
    guardrail: &ToolPermissionGuardrail,
    data: &mut Object,
    format: ResponseFormat,
    suppressed: &BTreeSet<String>,
    response: &mut GuardrailValue,
) -> Result<(), GuardrailFailure> {
    match format {
        ResponseFormat::OpenAi => post_call_openai(guardrail, data, suppressed, response),
        ResponseFormat::Anthropic => post_call_anthropic(guardrail, data, suppressed, response),
    }
}

fn post_call_openai(
    guardrail: &ToolPermissionGuardrail,
    data: &mut Object,
    suppressed: &BTreeSet<String>,
    response: &mut GuardrailValue,
) -> Result<(), GuardrailFailure> {
    let GuardrailValue::Object(fields) = response else {
        return Ok(());
    };
    if !should_run(guardrail, data, suppressed)? {
        return Ok(());
    }
    let calls = extract_response_tools(fields);
    if calls.is_empty() {
        tracing::debug!("Tool Permission Guardrail: No tool uses found");
        return Ok(());
    }
    tracing::debug!(
        count = calls.len(),
        "Tool Permission Guardrail: Found tool calls"
    );
    let denied = evaluate_tool_calls(guardrail, &calls)?;
    if denied.is_empty() {
        tracing::debug!("Tool Permission Guardrail Post-Call Hook: All tools allowed");
    } else {
        rewrite_openai_responses(fields, &denied);
    }
    record_applied_guardrail(data, &guardrail.args.name);
    Ok(())
}

fn post_call_anthropic(
    guardrail: &ToolPermissionGuardrail,
    data: &mut Object,
    suppressed: &BTreeSet<String>,
    response: &mut GuardrailValue,
) -> Result<(), GuardrailFailure> {
    // LiteLLM returns before the guardrail check when the body is not an Anthropic message with content blocks.
    let Some(content) = anthropic_content_blocks(response) else {
        return Ok(());
    };
    if !should_run(guardrail, data, suppressed)? {
        return Ok(());
    }
    let calls = extract_anthropic_tools(content);
    if calls.is_empty() {
        tracing::debug!("Tool Permission Guardrail: No tool uses found");
        return Ok(());
    }
    tracing::debug!(
        count = calls.len(),
        "Tool Permission Guardrail: Found tool calls"
    );
    let denied = evaluate_tool_calls(guardrail, &calls)?;
    let GuardrailValue::Object(fields) = response else {
        return Ok(());
    };
    if denied.is_empty() {
        tracing::debug!("Tool Permission Guardrail Post-Call Hook: All tools allowed");
    } else {
        rewrite_anthropic_responses(fields, &denied);
    }
    record_applied_guardrail(data, &guardrail.args.name);
    Ok(())
}

// LiteLLM's should_run_guardrail check for post_call, logged the way its hook logs the skip
fn should_run(
    guardrail: &ToolPermissionGuardrail,
    data: &Object,
    suppressed: &BTreeSet<String>,
) -> Result<bool, GuardrailFailure> {
    tracing::debug!("Tool Permission Guardrail Post-Call Hook: Checking response");
    if guardrail_base(guardrail).should_run_guardrail(
        data,
        GuardrailEventHooks::PostCall,
        suppressed,
    )? {
        return Ok(true);
    }
    tracing::debug!("Tool Permission Guardrail: Skipping check (not enabled)");
    Ok(false)
}

mod tests;
