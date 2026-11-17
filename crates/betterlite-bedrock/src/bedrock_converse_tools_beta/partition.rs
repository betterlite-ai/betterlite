use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::bedrock_converse_tools_beta::json::{as_object, as_text};

const SYSTEM_TOOL: &str = "systemTool";
const TYPE: &str = "type";
const TOOL_SEARCH_TYPES: [&str; 2] = [
    "tool_search_tool_regex_20251119",
    "tool_search_tool_bm25_20251119",
];

pub struct Partitioned {
    pub filtered: Vec<Json>,
    pub pre_formatted: Vec<Json>,
}

#[must_use]
pub fn partition_tools(tools: &[Json]) -> Partitioned {
    let mut filtered = Vec::with_capacity(tools.len());
    let mut pre_formatted = Vec::new();
    for tool in tools {
        let Some(fields) = as_object(tool) else {
            filtered.push(tool.clone());
            continue;
        };
        if fields.contains_key(SYSTEM_TOOL) {
            pre_formatted.push(tool.clone());
            continue;
        }
        if is_tool_search(fields) {
            continue;
        }
        filtered.push(tool.clone());
    }
    Partitioned {
        filtered,
        pre_formatted,
    }
}

// Converse has no tool-search equivalent, so LiteLLM drops these two versions without a word.
fn is_tool_search(fields: &IndexMap<String, Json>) -> bool {
    fields
        .get(TYPE)
        .and_then(as_text)
        .is_some_and(|kind| TOOL_SEARCH_TYPES.contains(&kind))
}
