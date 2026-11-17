use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::tool_permission::types::{is_tool_use_block, mapping_value, object_list};

use super::extraction::ToolCallInfo;

// LiteLLM's _get_anthropic_content_blocks: only a response that really is a mapping with a content list has blocks
#[must_use]
pub fn anthropic_content_blocks(response: &GuardrailValue) -> Option<&[GuardrailValue]> {
    if !matches!(response, GuardrailValue::Object(_)) {
        return None;
    }
    mapping_value(response, "content").and_then(object_list)
}

// LiteLLM's _anthropic_tool_use_to_tool_call: a tool_use needs a name, its input becomes the arguments, an absent id acts empty
#[must_use]
pub fn anthropic_to_tool_call(block: &GuardrailValue) -> Option<ToolCallInfo> {
    if !is_tool_use_block(block) {
        return None;
    }
    let name = match mapping_value(block, "name") {
        Some(GuardrailValue::Text(name)) if !name.is_empty() => name.clone(),
        None | Some(_) => return None,
    };
    let id = match mapping_value(block, "id") {
        Some(GuardrailValue::Text(id)) => id.clone(),
        None | Some(_) => String::new(),
    };
    // LiteLLM json.dumps a dict input and uses "{}" for anything else; the object itself parses the same way
    let arguments = match mapping_value(block, "input") {
        Some(GuardrailValue::Object(_)) => mapping_value(block, "input").cloned(),
        None | Some(_) => Some(GuardrailValue::Text("{}".to_owned())),
    };
    Some(ToolCallInfo {
        id,
        kind: Some("function".to_owned()),
        name: Some(name),
        arguments,
    })
}

mod tests;
