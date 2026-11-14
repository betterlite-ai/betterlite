use std::collections::BTreeSet;

use betterlite_core::api::codex_history_tools::drop_unsupported_tools;
use betterlite_prompts::arguments::json::Json;

use crate::mantle_responses::error::MantleResponsesError;
use crate::mantle_responses::repr::quoted_list;

pub const SUPPORTED_TOOL_TYPES: [&str; 6] = [
    "custom",
    "function",
    "mcp",
    "namespace",
    "tool_search",
    "web_search",
];

pub(crate) const SUPPORTED_TOOL_TYPES_REPR: &str =
    "['custom', 'function', 'mcp', 'namespace', 'tool_search', 'web_search']";

pub fn filter_unsupported_tools(tools: &[Json]) -> Result<Vec<Json>, MantleResponsesError> {
    let supported: BTreeSet<String> = SUPPORTED_TOOL_TYPES
        .map(str::to_owned)
        .into_iter()
        .collect();
    let mut raw = Vec::with_capacity(tools.len());
    for tool in tools {
        raw.push(serde_json::to_value(tool).map_err(MantleResponsesError::Encode)?);
    }
    let (kept, dropped) = drop_unsupported_tools(&raw, &supported);
    if !dropped.is_empty() {
        tracing::warn!(
            "Bedrock Mantle Responses API: dropping unsupported tool type(s) {} (supported: {}).",
            quoted_list(&dropped),
            SUPPORTED_TOOL_TYPES_REPR
        );
    }
    let mut filtered = Vec::with_capacity(kept.len());
    for tool in kept {
        filtered.push(serde_json::from_value(tool).map_err(MantleResponsesError::Encode)?);
    }
    Ok(filtered)
}
