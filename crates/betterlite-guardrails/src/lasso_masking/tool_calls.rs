use std::collections::BTreeMap;

use super::wire::{ToolCall, ToolUseBlock};

// A call with no match, or a matched block with no input, passes through unchanged so every other field survives verbatim.
#[must_use]
pub(super) fn update_tool_calls_from_masked(
    calls: &[ToolCall],
    masked: &BTreeMap<String, ToolUseBlock>,
) -> Vec<ToolCall> {
    let mut updated = Vec::with_capacity(calls.len());
    for call in calls {
        updated.push(apply_masked_input(call, masked));
    }
    updated
}

fn apply_masked_input(call: &ToolCall, masked: &BTreeMap<String, ToolUseBlock>) -> ToolCall {
    let mut updated = call.clone();
    let Some(id) = call.id.as_ref() else {
        return updated;
    };
    let Some(block) = masked.get(id) else {
        return updated;
    };
    let Some(input) = block.input.as_ref() else {
        return updated;
    };
    let arguments = serde_json::to_string(input).unwrap_or_default();
    let mut function = updated.function.unwrap_or_default();
    function.arguments = Some(arguments);
    updated.function = Some(function);
    updated
}
