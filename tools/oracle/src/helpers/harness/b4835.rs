use serde_json::{Value, json};
use std::collections::BTreeSet;

use crate::helpers::{Helper, HelperError, HelperResult};

pub(crate) struct Op;

#[derive(serde::Deserialize)]
pub(crate) struct Input {
    fixture: String,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let contents = std::fs::read_to_string(&input.fixture).map_err(|error| {
            Box::new(HelperError {
                class: "FixtureRead".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })?;
        let mut thread_id = None;
        let mut final_text = String::new();
        let mut error = None;
        let mut failed = false;
        let mut started = BTreeSet::new();
        let mut events = Vec::new();
        for line in contents.lines().filter(|line| !line.is_empty()) {
            let event: Value = serde_json::from_str(line).map_err(|parse_error| {
                Box::new(HelperError {
                    class: "FixtureParse".to_owned(),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: parse_error.to_string(),
                })
            })?;
            match event.get("type").and_then(Value::as_str) {
                Some("thread.started") => {
                    thread_id = event
                        .get("thread_id")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                }
                Some("error") => {
                    error = event
                        .get("message")
                        .and_then(Value::as_str)
                        .map_or_else(|| Some("codex reported an error".to_owned()), |value| {
                            Some(value.to_owned())
                        });
                }
                Some("turn.failed") => {
                    failed = true;
                    error = event
                        .get("error")
                        .and_then(|value| value.get("message").or(Some(value)))
                        .and_then(Value::as_str)
                        .map_or_else(|| Some("codex turn failed".to_owned()), |value| {
                            Some(value.to_owned())
                        });
                }
                Some("item.started") | Some("item.updated") | Some("item.completed") => {
                    let Some(item) = event.get("item").and_then(Value::as_object) else {
                        continue;
                    };
                    let kind = item.get("type").and_then(Value::as_str).unwrap_or_default();
                    let id = item
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let completed = event.get("type").and_then(Value::as_str) == Some("item.completed");
                    if kind == "agent_message" && completed {
                        final_text = item
                            .get("text")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned();
                        events.push(json!({"kind": "text", "id": id}));
                    } else if kind == "reasoning" && completed {
                        events.push(json!({"kind": "reasoning", "id": id}));
                    } else if [
                        "command_execution",
                        "file_change",
                        "web_search",
                        "mcp_tool_call",
                    ]
                    .contains(&kind)
                    {
                        if started.insert(id.to_owned()) {
                            events.push(json!({"kind": "tool_call", "id": id}));
                        }
                        if completed {
                            events.push(json!({"kind": "tool_result", "id": id}));
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(json!({
            "thread_id": thread_id,
            "final_text": final_text,
            "error": error,
            "failed": failed,
            "events": events,
        }))
    }
}
