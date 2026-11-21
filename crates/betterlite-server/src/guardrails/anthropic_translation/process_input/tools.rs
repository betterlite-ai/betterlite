use betterlite_anthropic::tools::map_tool_helper;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::api::messages_content_primitives::tools::is_provider_native_tool_dict;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailToolParam;
use betterlite_guardrails::guardrail_message_scope::Object;
use betterlite_guardrails::guardrail_message_scope::merge_tools::merge_returned_tools;
use betterlite_guardrails::guardrail_message_scope::tool_names::anthropic_tool_names;
use betterlite_prompts::arguments::json::{Json as PromptJson, Object as PromptObject};

use crate::guardrails::anthropic_translation::input_scan::serde_convert;

use super::TOOLS;
use super::error::ProcessInputError;

pub(super) fn write_returned_tools(
    data: &mut Object,
    returned: &[GuardrailToolParam],
    scan_only: bool,
) -> Result<(), ProcessInputError> {
    let converted_tools = anthropic_tools(returned)?;
    let request_tools: Vec<GuardrailValue> = match data.get(TOOLS) {
        Some(GuardrailValue::Array(tools)) => tools.clone(),
        Some(GuardrailValue::Null) | None => Vec::new(),
        Some(_) => {
            return Err(ProcessInputError::FieldType {
                field: TOOLS,
                expected: "an array",
            });
        }
    };
    let merged = if scan_only {
        merge_returned_tools(&request_tools, &converted_tools, |tool| {
            anthropic_tool_names(tool)
                .first()
                .copied()
                .map(str::to_owned)
        })
    } else {
        native_tools(&request_tools, converted_tools)?
    };
    data.insert(TOOLS.to_owned(), GuardrailValue::Array(merged));
    Ok(())
}

fn anthropic_tools(
    returned: &[GuardrailToolParam],
) -> Result<Vec<GuardrailValue>, ProcessInputError> {
    let mut anthropic = Vec::with_capacity(returned.len());
    for tool in returned {
        let chat_tool: PromptJson = serde_convert(tool, "returned tool")?;
        let (converted, _mcp_server) = map_tool_helper(&chat_tool)?;
        if let Some(tool_value) = converted {
            anthropic.push(serde_convert::<_, GuardrailValue>(
                &tool_value,
                "returned tool",
            )?);
        }
    }
    Ok(anthropic)
}

fn native_tools(
    request_tools: &[GuardrailValue],
    returned: Vec<GuardrailValue>,
) -> Result<Vec<GuardrailValue>, ProcessInputError> {
    let mut native: Vec<GuardrailValue> = Vec::new();
    for tool in request_tools {
        let fields: PromptObject = serde_convert(tool, "request tool")?;
        if is_provider_native_tool_dict(&fields) {
            native.push(tool.clone());
        }
    }
    native.extend(returned);
    Ok(native)
}
