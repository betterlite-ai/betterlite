use std::collections::BTreeMap;

use betterlite_db::mcp_stored_contracts::PinnedMCPTool;

use super::PinnedCatalogDrift;
use super::types::MCPTool;

#[must_use]
pub fn pin_tool_catalog(
    tools: &[MCPTool],
    pinned_tools: &BTreeMap<String, PinnedMCPTool>,
) -> (Vec<MCPTool>, Option<PinnedCatalogDrift>) {
    let upstream: BTreeMap<&str, &MCPTool> = tools
        .iter()
        .map(|tool| (tool.name.as_str(), tool))
        .collect();

    let (added, removed, changed) = drift_names(&upstream, pinned_tools);
    let served = served_tools(tools, pinned_tools, &changed);

    let has_drift = !added.is_empty() || !removed.is_empty() || !changed.is_empty();
    let drift = has_drift.then(|| PinnedCatalogDrift {
        added: sorted(added),
        removed: sorted(removed),
        changed: sorted(changed),
    });
    (served, drift)
}

fn drift_names(
    upstream: &BTreeMap<&str, &MCPTool>,
    pinned_tools: &BTreeMap<String, PinnedMCPTool>,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let added = upstream
        .keys()
        .filter(|name| !pinned_tools.contains_key(**name))
        .map(|name| (*name).to_owned())
        .collect();
    let removed = pinned_tools
        .keys()
        .filter(|name| !upstream.contains_key(name.as_str()))
        .cloned()
        .collect();
    let changed = upstream
        .iter()
        .filter(|(name, tool)| {
            pinned_tools
                .get(**name)
                .is_some_and(|pinned| drifted(tool, pinned))
        })
        .map(|(name, _)| (*name).to_owned())
        .collect();
    (added, removed, changed)
}

fn served_tools(
    tools: &[MCPTool],
    pinned_tools: &BTreeMap<String, PinnedMCPTool>,
    changed: &[String],
) -> Vec<MCPTool> {
    tools
        .iter()
        .filter(|tool| pinned_tools.contains_key(tool.name.as_str()))
        .map(|tool| {
            if changed.contains(&tool.name) {
                pinned_tool(tool, &pinned_tools[&tool.name])
            } else {
                tool.clone()
            }
        })
        .collect()
}

fn drifted(tool: &MCPTool, pinned: &PinnedMCPTool) -> bool {
    tool.description.as_deref().unwrap_or("") != pinned.as_ref().description
        || tool.input_schema != pinned.as_ref().input_schema
}

fn pinned_tool(tool: &MCPTool, pinned: &PinnedMCPTool) -> MCPTool {
    let description = pinned.as_ref().description.clone();
    MCPTool {
        description: (!description.is_empty()).then_some(description),
        input_schema: pinned.as_ref().input_schema.clone(),
        ..tool.clone()
    }
}

fn sorted(mut names: Vec<String>) -> Vec<String> {
    names.sort();
    names
}
