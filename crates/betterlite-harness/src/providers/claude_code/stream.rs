use super::config::normalize_tool_name;
use crate::events::Event;
use crate::payloads::last_json_object;
use crate::types::JsonObject;

#[derive(Clone, Debug, Default)]
pub struct ClaudeCodeStreamState {
    pub session_id: Option<String>,
    pub text_parts: Vec<String>,
    pub result_seen: bool,
    pub result_text: Option<String>,
    pub is_error: bool,
    pub errors: Vec<String>,
    pub structured_output: Option<serde_json::Value>,
}

impl ClaudeCodeStreamState {
    pub fn final_text(&self) -> String {
        let final_text = self
            .result_text
            .clone()
            .unwrap_or_else(|| self.text_parts.join(""));
        final_text
    }
    pub fn structured_output_json(&self) -> String {
        let output = self
            .structured_output
            .as_ref()
            .filter(|value| value.is_object())
            .and_then(|value| serde_json::to_string(value).ok())
            .or_else(|| last_json_object(&self.final_text()))
            .unwrap_or_default();
        output
    }
}

pub(super) fn transform(line: &JsonObject, state: &mut ClaudeCodeStreamState) -> Vec<Event> {
    match line.get("type").and_then(serde_json::Value::as_str) {
        Some("assistant") => assistant(line, state),
        Some("user") => user(line),
        Some("system") => system(line, state),
        Some("result") => result(line, state),
        _ => Vec::new(),
    }
}

fn blocks(line: &JsonObject) -> Vec<serde_json::Value> {
    let Some(message) = line.get("message") else {
        return Vec::new();
    };
    match message.get("content") {
        Some(serde_json::Value::String(text)) => {
            vec![serde_json::json!({"type":"text", "text":text})]
        }
        Some(serde_json::Value::Array(items)) => items.clone(),
        _ => Vec::new(),
    }
}
fn value_text(value: &serde_json::Value) -> String {
    let text = value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_owned);
    text
}
fn json_object(value: Option<&serde_json::Value>) -> JsonObject {
    let object =
        value
            .and_then(serde_json::Value::as_object)
            .map_or_else(JsonObject::new, |object| {
                object
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect()
            });
    object
}

fn assistant(line: &JsonObject, state: &mut ClaudeCodeStreamState) -> Vec<Event> {
    if line
        .get("parent_tool_use_id")
        .is_some_and(|value| !value.is_null())
        || line
            .get("message")
            .and_then(|message| message.get("model"))
            .and_then(serde_json::Value::as_str)
            == Some("<synthetic>")
    {
        return Vec::new();
    }
    let mut events = Vec::new();
    for block in blocks(line) {
        match block.get("type").and_then(serde_json::Value::as_str) {
            Some("text") => {
                if let Some(text) = block
                    .get("text")
                    .and_then(serde_json::Value::as_str)
                    .filter(|text| !text.is_empty())
                {
                    state.text_parts.push(text.to_owned());
                    events.push(Event::Text {
                        delta: text.to_owned(),
                    });
                }
            }
            Some("thinking") => {
                if let Some(text) = block
                    .get("thinking")
                    .and_then(serde_json::Value::as_str)
                    .filter(|text| !text.is_empty())
                {
                    events.push(Event::Reasoning {
                        delta: text.to_owned(),
                    });
                }
            }
            Some("tool_use") => {
                let native = block.get("name").map_or_else(String::new, value_text);
                events.push(Event::ToolCall {
                    id: block.get("id").map_or_else(String::new, value_text),
                    name: normalize_tool_name(&native),
                    native_name: native.clone(),
                    input: json_object(block.get("input")),
                    builtin: !native.starts_with("mcp__"),
                });
            }
            _ => {}
        }
    }
    events
}

fn user(line: &JsonObject) -> Vec<Event> {
    if line
        .get("parent_tool_use_id")
        .is_some_and(|value| !value.is_null())
    {
        return Vec::new();
    }
    blocks(line)
        .into_iter()
        .filter(|block| {
            block.get("type").and_then(serde_json::Value::as_str) == Some("tool_result")
        })
        .map(|block| Event::ToolResult {
            id: block
                .get("tool_use_id")
                .map_or_else(String::new, value_text),
            output: stringify(block.get("content")),
            is_error: block
                .get("is_error")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
        })
        .collect()
}
fn system(line: &JsonObject, state: &mut ClaudeCodeStreamState) -> Vec<Event> {
    if line.get("subtype").and_then(serde_json::Value::as_str) == Some("init") {
        if let Some(id) = line.get("session_id") {
            state.session_id = Some(value_text(id));
        }
    }
    if line.get("subtype").and_then(serde_json::Value::as_str) == Some("compact_boundary") {
        return vec![Event::Compaction {
            tokens_before: line
                .get("compact_metadata")
                .and_then(|value| value.get("pre_tokens"))
                .and_then(serde_json::Value::as_u64),
            tokens_after: None,
        }];
    }
    Vec::new()
}
fn result(line: &JsonObject, state: &mut ClaudeCodeStreamState) -> Vec<Event> {
    state.result_seen = true;
    state.is_error = line
        .get("is_error")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    state.result_text = line
        .get("result")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    state.errors = line
        .get("errors")
        .and_then(serde_json::Value::as_array)
        .map_or_else(Vec::new, |items| items.iter().map(value_text).collect());
    state.structured_output = line.get("structured_output").cloned();
    if let Some(id) = line.get("session_id") {
        state.session_id = Some(value_text(id));
    }
    Vec::new()
}
pub(super) fn turn_error(
    state: &ClaudeCodeStreamState,
    exit_code: i32,
    stderr_tail: &[String],
) -> Option<String> {
    if exit_code == 0 && state.result_seen && !state.is_error {
        return None;
    }
    let reason = state
        .result_text
        .clone()
        .or_else(|| (!state.errors.is_empty()).then(|| state.errors.join("; ")))
        .unwrap_or_else(|| {
            if state.result_seen {
                "unknown error".to_owned()
            } else {
                "no result event".to_owned()
            }
        });
    let message = format!("claude exited with code {exit_code}: {reason}");
    let tail = stderr_tail
        .iter()
        .filter(|line| !line.trim().is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    Some(if tail.is_empty() {
        message
    } else {
        format!("{message}\nstderr:\n{tail}")
    })
}
fn stringify(value: Option<&serde_json::Value>) -> String {
    match value {
        None | Some(serde_json::Value::Null) => String::new(),
        Some(serde_json::Value::String(text)) => text.clone(),
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .map(|item| {
                if item.get("type").and_then(serde_json::Value::as_str) == Some("text") {
                    item.get("text").map_or_else(String::new, value_text)
                } else {
                    item.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Some(value) => value.to_string(),
    }
}
