use std::collections::BTreeSet;

use betterlite_harness::events::Event;
use serde_json::json;

use crate::helpers::HelperResult;

pub(super) fn encode(event: &Event, started: &mut BTreeSet<String>) -> HelperResult {
    match event {
        Event::Text { delta } => Ok(json!({"kind": "text", "delta": delta})),
        Event::Reasoning { delta } => Ok(json!({"kind": "reasoning", "delta": delta})),
        Event::ToolCall {
            id,
            name,
            native_name,
            input,
            builtin,
        } => {
            started.insert(id.clone());
            Ok(json!({
                "kind": "tool_call", "id": id, "name": name,
                "native_name": native_name, "input": input, "builtin": builtin,
            }))
        }
        Event::ToolResult {
            id,
            output,
            is_error,
        } => {
            if !started.contains(id) {
                return Err(super::failure("EventOrder", "ToolResult preceded ToolCall"));
            }
            Ok(json!({"kind": "tool_result", "id": id, "output": output, "is_error": is_error}))
        }
        Event::FileChange(_)
        | Event::Compaction { .. }
        | Event::Approval(_)
        | Event::Done { .. } => Err(super::failure("UnexpectedEvent", "Unexpected Codex event")),
    }
}
