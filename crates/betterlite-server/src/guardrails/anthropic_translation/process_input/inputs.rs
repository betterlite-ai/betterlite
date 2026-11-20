use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::api::messages_content_primitives::tools::is_provider_native_tool_dict;
use betterlite_guardrails::generic_guardrail_contracts::{GuardrailMessage, GuardrailToolParam};
use betterlite_guardrails::guardrail_base::inputs::GuardrailApiInputs;
use betterlite_guardrails::guardrail_message_scope::Object;
use betterlite_prompts::arguments::json::Object as PromptObject;

use crate::guardrails::anthropic_translation::input_scan::serde_convert;

use super::error::ProcessInputError;
use super::{MODEL, RequestScan};

pub(super) fn tools_to_check(
    scan_only: bool,
    tools: &[PromptObject],
) -> Result<Vec<GuardrailToolParam>, ProcessInputError> {
    if scan_only {
        return Ok(Vec::new());
    }
    checked_tools(tools)
}

fn checked_tools(tools: &[PromptObject]) -> Result<Vec<GuardrailToolParam>, ProcessInputError> {
    let mut checked = Vec::new();
    for tool in tools {
        if !is_provider_native_tool_dict(tool) {
            checked.push(serde_convert::<_, GuardrailToolParam>(
                tool,
                "request tool",
            )?);
        }
    }
    Ok(checked)
}

pub(super) fn guardrail_inputs(
    data: &Object,
    scanned: &RequestScan,
    structured_messages: &[GuardrailMessage],
    tools: Vec<GuardrailToolParam>,
) -> Result<GuardrailApiInputs, ProcessInputError> {
    let mut inputs = GuardrailApiInputs {
        texts: Some(scanned.texts.clone()),
        images: None,
        tools: None,
        tool_calls: None,
        structured_messages: None,
        model: None,
        stream_holdback_chars: None,
    };
    if !scanned.images.is_empty() {
        inputs.images = Some(scanned.images.clone());
    }
    if !scanned.tool_calls.is_empty() {
        let mut converted = Vec::with_capacity(scanned.tool_calls.len());
        for tool_call in &scanned.tool_calls {
            converted.push(serde_convert::<_, GuardrailValue>(tool_call, "tool call")?);
        }
        inputs.tool_calls = Some(converted);
    }
    if !tools.is_empty() {
        inputs.tools = Some(tools);
    }
    if !structured_messages.is_empty() {
        inputs.structured_messages = Some(structured_messages.to_vec());
    }
    if let Some(GuardrailValue::Text(model)) = data.get(MODEL)
        && !model.is_empty()
    {
        inputs.model = Some(model.clone());
    }
    Ok(inputs)
}
