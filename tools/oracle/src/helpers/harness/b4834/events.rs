use std::collections::BTreeSet;

use betterlite_harness::events::Event;
use serde_json::json;

use crate::helpers::{HelperError, HelperResult};

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
        } => tool_result(id, output, *is_error, started),
        Event::Compaction {
            tokens_before,
            tokens_after,
        } => Ok(json!({
            "kind": "compaction", "tokens_before": tokens_before, "tokens_after": tokens_after,
        })),
        Event::FileChange(_) | Event::Approval(_) | Event::Done { .. } => {
            Err(Box::new(HelperError {
                class: "UnexpectedEvent".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: "Unexpected Claude Code event".to_owned(),
            }))
        }
    }
}

fn tool_result(id: &str, output: &str, is_error: bool, started: &BTreeSet<String>) -> HelperResult {
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
