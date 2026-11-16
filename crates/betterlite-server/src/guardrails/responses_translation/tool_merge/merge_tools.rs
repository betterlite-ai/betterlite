use std::collections::{BTreeMap, BTreeSet};

use betterlite_core::api::responses_tool_namespaces::responses_tools::chat_tools_to_responses;
use betterlite_core::api::responses_tool_namespaces::values::{Fields, tool_type};
use betterlite_models::decisions::Json;

use super::as_tool::{namespace_members, validated_tools};
use super::error::ToolMergeError;
use super::group::{CHAT_MEMBER_TYPES, FlattenedGroup};
use super::indexed_key::{IndexedKey, indexed_keys};
use super::rebuild_namespace::rebuilt_namespace;

mod tests;

fn chat_tools_as_responses(tools: &[&Fields]) -> Result<Vec<Fields>, ToolMergeError> {
    let chat_tools: Vec<Json> = tools
        .iter()
        .map(|tool| Json::Object((*tool).clone()))
        .collect();
    Ok(chat_tools_to_responses(&chat_tools)?)
}

// An unchanged group keeps its original tool, a namespace is rebuilt in place, and any other tool becomes whatever the guardrail returned for it.
fn merged_original(
    original: &Fields,
    group: &FlattenedGroup<'_>,
) -> Result<Vec<Fields>, ToolMergeError> {
    if group.keys.is_empty() {
        return Ok(vec![original.clone()]);
    }
    let guardrailed_group: Vec<&Fields> = group
        .keys
        .iter()
        .filter_map(|key| group.guardrailed.get(key).copied())
        .collect();
    if guardrailed_group.iter().copied().eq(group.tools.iter()) {
        return Ok(vec![original.clone()]);
    }
    let members = if tool_type(original) == "namespace" {
        namespace_members(original)
    } else {
        Vec::new()
    };
    let chat_members = members
        .iter()
        .filter(|member| CHAT_MEMBER_TYPES.contains(&tool_type(member)))
        .count();
    if !members.is_empty() && chat_members == group.tools.len() {
        return rebuilt_namespace(original, &members, group);
    }
    if guardrailed_group.is_empty() {
        return Ok(Vec::new());
    }
    chat_tools_as_responses(&guardrailed_group)
}

// Tools the guardrail returned under a key no flattened tool owned are appended after the merged originals.
pub fn merge_guardrailed_tools(
    original_tools: &[Fields],
    flattened_groups: &[Vec<Fields>],
    guardrailed_tools: &[Json],
) -> Result<Vec<Fields>, ToolMergeError> {
    let guardrailed = validated_tools(guardrailed_tools);
    let flattened: Vec<&Fields> = flattened_groups.iter().flatten().collect();
    let flattened_keys = indexed_keys(&flattened)?;
    let guardrailed_keys = indexed_keys(&guardrailed)?;
    let guardrailed_by_key: BTreeMap<IndexedKey, &Fields> = guardrailed_keys
        .iter()
        .cloned()
        .zip(guardrailed.iter().copied())
        .collect();
    let mut merged = Vec::new();
    let mut start = 0_usize;
    for (original, flattened_group) in original_tools.iter().zip(flattened_groups) {
        let end = start.saturating_add(flattened_group.len());
        let group = FlattenedGroup {
            tools: flattened_group,
            keys: flattened_keys.get(start..end).unwrap_or_default(),
            guardrailed: &guardrailed_by_key,
        };
        merged.extend(merged_original(original, &group)?);
        start = end;
    }
    let owned_keys: BTreeSet<&IndexedKey> = flattened_keys.iter().collect();
    let new_tools: Vec<&Fields> = guardrailed_keys
        .iter()
        .zip(&guardrailed)
        .filter(|(key, _)| !owned_keys.contains(key))
        .map(|(_, tool)| *tool)
        .collect();
    merged.extend(chat_tools_as_responses(&new_tools)?);
    Ok(merged)
}
