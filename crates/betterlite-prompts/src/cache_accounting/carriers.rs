use crate::arguments::json::Json;
use crate::cache_accounting::constants::{BREAKPOINT_BLOCK_TYPES, CACHE_BREAKPOINT_KEYS};

// The key has to hold a value: an explicit null is no breakpoint, as an explicit null is no cache_control.
#[must_use]
pub fn carries_cache_breakpoint(block: &Json) -> bool {
    block.as_object().is_some_and(|fields| {
        CACHE_BREAKPOINT_KEYS.iter().any(|key| {
            fields
                .get(*key)
                .is_some_and(|value| !matches!(value, Json::Null))
        })
    })
}

// Anthropic tools carry the mark at the top level, OpenAI tools inside their function object.
#[must_use]
pub fn tool_carries_cache_breakpoint(tool: &Json) -> bool {
    carries_cache_breakpoint(tool)
        || tool
            .as_object()
            .and_then(|fields| fields.get("function"))
            .is_some_and(carries_cache_breakpoint)
}

// The chat transform rewrites the tool search tools and drops their cache control on the way.
#[must_use]
pub fn drops_tool_cache_control(tool: &Json) -> bool {
    tool.as_object()
        .and_then(|fields| fields.get("type"))
        .and_then(Json::as_str)
        .is_some_and(|kind| {
            matches!(
                kind,
                "tool_search_tool_regex_20251119" | "tool_search_tool_bm25_20251119"
            )
        })
}

#[must_use]
pub fn accepts_cache_breakpoint(block: &Json) -> bool {
    block
        .as_object()
        .and_then(|fields| fields.get("type"))
        .and_then(Json::as_str)
        .is_some_and(|kind| BREAKPOINT_BLOCK_TYPES.contains(&kind))
}
