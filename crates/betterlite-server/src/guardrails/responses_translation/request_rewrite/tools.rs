use std::slice;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::api::responses_tool_namespaces::namespace_names::check_collisions;
use betterlite_core::api::responses_tool_namespaces::responses_tools::responses_tools_to_chat;
use betterlite_core::api::responses_tool_namespaces::values::Fields;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailToolParam;
use betterlite_guardrails::guardrail_message_scope::Object;
use betterlite_models::decisions::Json;

use super::error::RequestRewriteError;
use super::serde_convert;
use crate::guardrails::responses_translation::tool_merge::merge_tools::merge_guardrailed_tools;

pub fn original_tools(data: &Object) -> Result<Vec<Fields>, RequestRewriteError> {
    let Some(GuardrailValue::Array(tools)) = data.get("tools") else {
        return Ok(Vec::new());
    };
    tools
        .iter()
        .map(|tool| serde_convert(tool, "request tool"))
        .collect()
}

// One group of chat tools per request tool, the shape LiteLLM's per-tool chat forms have
pub fn flattened_tool_groups(
    original_tools: &[Fields],
) -> Result<Vec<Vec<Fields>>, RequestRewriteError> {
    let tools: Vec<Json> = original_tools.iter().cloned().map(Json::Object).collect();
    check_collisions(&tools).map_err(|source| RequestRewriteError::Tools { source })?;
    let mut groups = Vec::with_capacity(tools.len());
    for tool in &tools {
        let conversion = responses_tools_to_chat(slice::from_ref(tool))
            .map_err(|source| RequestRewriteError::Tools { source })?;
        for warning in &conversion.warnings {
            tracing::warn!("{warning}");
        }
        let mut group = Vec::with_capacity(conversion.chat_tools.len());
        for chat_tool in &conversion.chat_tools {
            group.push(serde_convert(chat_tool, "chat tool")?);
        }
        groups.push(group);
    }
    Ok(groups)
}

pub fn apply_guardrailed_tools_to_data(
    data: &mut Object,
    original_tools: &[Fields],
    flattened_tool_groups: &[Vec<Fields>],
    returned_tools: &[GuardrailToolParam],
) -> Result<(), RequestRewriteError> {
    let returned: Vec<Json> = serde_convert(returned_tools, "returned tools")?;
    let merged = merge_guardrailed_tools(original_tools, flattened_tool_groups, &returned)
        .map_err(|source| RequestRewriteError::ToolMerge { source })?;
    data.insert("tools".to_owned(), serde_convert(&merged, "merged tools")?);
    Ok(())
}
