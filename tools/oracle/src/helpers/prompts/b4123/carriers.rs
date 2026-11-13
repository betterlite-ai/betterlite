use betterlite_prompts::arguments::json::Json;
use betterlite_prompts::cache_accounting::carriers::{
    accepts_cache_breakpoint, carries_cache_breakpoint, drops_tool_cache_control,
    tool_carries_cache_breakpoint,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Call {
    #[serde(default)]
    blocks: Vec<Json>,
    #[serde(default)]
    tools: Vec<Json>,
}

pub(crate) fn carriers(call: &Call) -> Value {
    let blocks: Vec<Value> = call
        .blocks
        .iter()
        .map(|block| {
            json!({
                "carries": carries_cache_breakpoint(block),
                "accepts": accepts_cache_breakpoint(block),
            })
        })
        .collect();
    let tools: Vec<Value> = call
        .tools
        .iter()
        .map(|tool| {
            json!({
                "carries": tool_carries_cache_breakpoint(tool),
                "drops": drops_tool_cache_control(tool),
            })
        })
        .collect();
    json!({ "blocks": blocks, "tools": tools })
}
