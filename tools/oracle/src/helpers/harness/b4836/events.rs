use std::collections::BTreeSet;

use betterlite_harness::events::Event;
use serde_json::json;

use crate::helpers::{HelperError, HelperResult};

pub(super) fn encode(event: &Event, started: &mut BTreeSet<String>) -> HelperResult {
    match event {
        Event::Text { delta } => Ok(json!({"kind": "text", "delta": delta})),
        Event::Reasoning { delta } => Ok(json!({"kind": "reasoning", "delta": delta})),
        Event::ToolCall { .. } => Ok(encode_tool_call(event, started)),
        Event::ToolResult {
            id,
            output,
            is_error,
        } => encode_tool_result(id, output, *is_error, started),
        Event::FileChange(_)
        | Event::Compaction { .. }
        | Event::Approval(_)
        | Event::Done { .. } => Err(Box::new(HelperError {
            class: "UnexpectedEvent".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: "Unexpected OpenCode event".to_owned(),
        })),
    }
}

fn encode_tool_call(event: &Event, started: &mut BTreeSet<String>) -> serde_json::Value {
    let Event::ToolCall {
        id,
        name,
        native_name,
        input,
        builtin,
    } = event
    else {
        return json!(null);
    };
    started.insert(id.to_owned());
    json!({"kind": "tool_call", "id": id, "name": name, "native_name": native_name, "input": input, "builtin": builtin})
}

fn encode_tool_result(
    id: &str,
    output: &str,
    is_error: bool,
    started: &BTreeSet<String>,
) -> HelperResult {
    if !started.contains(id) {
        return Err(Box::new(HelperError {
            class: "EventOrder".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: "ToolResult preceded ToolCall".to_owned(),
        }));
    }
    Ok(json!({"kind": "tool_result", "id": id, "output": output, "is_error": is_error}))
}
