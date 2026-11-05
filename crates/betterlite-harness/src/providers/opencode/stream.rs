use crate::events::Event;
use crate::payloads::python_json::python_json_text;
use crate::types::JsonObject;

use super::wire::{Line, ToolPart};

const BUILTIN_TOOLS: [&str; 22] = [
    "read",
    "write",
    "edit",
    "multiedit",
    "patch",
    "apply_patch",
    "bash",
    "glob",
    "grep",
    "list",
    "webfetch",
    "websearch",
    "task",
    "todowrite",
    "todoread",
    "skill",
    "invalid",
    "question",
    "lsp",
    "codesearch",
    "plan_enter",
    "plan_exit",
];

#[derive(Clone, Debug, Default)]
pub struct OpenCodeStreamState {
    pub session_id: Option<String>,
    pub final_text: String,
    pub error: Option<String>,
    pub step_texts: Vec<String>,
}

fn tool_input(part: &ToolPart) -> JsonObject {
    match &part.state.input {
        Some(serde_json::Value::Object(entries)) => entries.clone().into_iter().collect(),
        Some(value) => JsonObject::from([(String::from("input"), value.clone())]),
        None => JsonObject::new(),
    }
}

fn tool_events(part: ToolPart) -> Vec<Event> {
    let native = part
        .tool
        .as_deref()
        .filter(|tool| !tool.is_empty())
        .unwrap_or_default()
        .to_owned();
    let call_id = part
        .call_id
        .as_ref()
        .filter(|id| !id.is_empty())
        .cloned()
        .or_else(|| part.id.as_ref().filter(|id| !id.is_empty()).cloned())
        .unwrap_or_default();
    let call = Event::ToolCall {
        id: call_id.clone(),
        name: super::permissions::normalize_tool_name(&native),
        native_name: native.clone(),
        input: tool_input(&part),
        builtin: BUILTIN_TOOLS.contains(&native.as_str()),
    };
    vec![call, tool_result(part, &native, call_id)]
}

fn tool_result(part: ToolPart, native: &str, call_id: String) -> Event {
    let (output, is_error) = if part
        .state
        .status
        .as_ref()
        .is_some_and(|status| status == "error")
    {
        (failed_tool_output(part.state), true)
    } else {
        (tool_output(part.state), native == "invalid")
    };
    Event::ToolResult {
        id: call_id,
        output,
        is_error,
    }
}

fn tool_output(state: super::wire::ToolState) -> String {
    match state.output {
        Some(serde_json::Value::String(text)) => text,
        Some(value) => python_json_text(&value).unwrap_or_else(|_| value.to_string()),
        None => String::from("null"),
    }
}

fn failed_tool_output(state: super::wire::ToolState) -> String {
    if let Some(error) = state.error.as_ref().filter(|error| !error.is_empty()) {
        return error.clone();
    }
    if let Some(value) = state.output
        && super::wire::is_truthy(&value)
    {
        return super::wire::value_to_string(&value);
    }
    "tool failed".to_owned()
}

pub(super) fn transform_line(line: &JsonObject, state: &mut OpenCodeStreamState) -> Vec<Event> {
    let wire = super::wire::parse(line);
    if state.session_id.is_none() {
        state.session_id = wire.session_id().map(str::to_owned);
    }
    transform_parsed_line(wire, state)
}

fn transform_parsed_line(wire: Line, state: &mut OpenCodeStreamState) -> Vec<Event> {
    match wire {
        Line::StepStart { .. } => {
            state.step_texts.clear();
            Vec::new()
        }
        Line::Text { part, .. } => text_event(part, state),
        Line::Reasoning { part, .. } => {
            let Some(delta) = part.text.filter(|text| !text.is_empty()) else {
                return Vec::new();
            };
            vec![Event::Reasoning { delta }]
        }
        Line::ToolUse { part, .. } => tool_events(part),
        Line::Error { error, .. } => {
            let message = error.as_ref().map_or_else(
                || "opencode reported an error".to_owned(),
                super::wire::error_message,
            );
            state.error = Some(match state.error.take() {
                Some(existing) => format!("{existing}\n{message}"),
                None => message,
            });
            Vec::new()
        }
        Line::Other => Vec::new(),
        Line::Malformed { line, .. } => {
            let message = format!("opencode line could not be parsed: {line}");
            state.error = Some(match state.error.take() {
                Some(existing) => format!("{existing}\n{message}"),
                None => message,
            });
            Vec::new()
        }
    }
}

fn text_event(part: super::wire::TextPart, state: &mut OpenCodeStreamState) -> Vec<Event> {
    let Some(text) = part.text.filter(|text| !text.is_empty()) else {
        return Vec::new();
    };
    state.step_texts.push(text.clone());
    state.final_text = state.step_texts.join("\n\n");
    vec![Event::Text { delta: text }]
}
