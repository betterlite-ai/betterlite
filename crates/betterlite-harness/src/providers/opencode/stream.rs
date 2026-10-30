use crate::events::Event;
use crate::payloads::python_json::python_json_text;
use crate::types::JsonObject;

use super::wire::{Line, ToolPart};

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
    let native = part.tool.clone().unwrap_or_default();
    let call_id = part
        .call_id
        .clone()
        .or_else(|| part.id.clone())
        .unwrap_or_default();
    let call = Event::ToolCall {
        id: call_id.clone(),
        name: super::permissions::normalize_tool_name(&native),
        native_name: native.clone(),
        input: tool_input(&part),
        builtin: matches!(
            native.as_str(),
            "read"
                | "write"
                | "edit"
                | "multiedit"
                | "patch"
                | "apply_patch"
                | "bash"
                | "glob"
                | "grep"
                | "list"
                | "webfetch"
                | "websearch"
                | "task"
                | "todowrite"
                | "todoread"
                | "skill"
                | "invalid"
                | "question"
                | "lsp"
                | "codesearch"
                | "plan_enter"
                | "plan_exit"
        ),
    };
    if part.state.status.as_deref() == Some("error") {
        let output = failed_tool_output(part.state);
        return vec![
            call,
            Event::ToolResult {
                id: call_id,
                output,
                is_error: true,
            },
        ];
    }
    let output = match part.state.output {
        Some(serde_json::Value::String(text)) => text,
        Some(value) => python_json_text(&value).unwrap_or_else(|_| value.to_string()),
        None => String::from("null"),
    };
    vec![
        call,
        Event::ToolResult {
            id: call_id,
            output,
            is_error: native == "invalid",
        },
    ]
}

fn failed_tool_output(state: super::wire::ToolState) -> String {
    if let Some(error) = state.error {
        return error;
    }
    if let Some(value) = state.output
        && let Some(text) = value.as_str()
    {
        return text.to_owned();
    }
    "tool failed".to_owned()
}

pub(super) fn transform_line(line: &JsonObject, state: &mut OpenCodeStreamState) -> Vec<Event> {
    let wire = super::wire::parse(line);
    if state.session_id.is_none() {
        state.session_id = wire.session_id().map(str::to_owned);
    }
    match wire {
        Line::StepStart { .. } => {
            state.step_texts.clear();
            Vec::new()
        }
        Line::Text { part, .. } => match part.text.filter(|text| !text.is_empty()) {
            Some(text) => {
                state.step_texts.push(text.clone());
                state.final_text = state.step_texts.join("\n\n");
                vec![Event::Text { delta: text }]
            }
            None => Vec::new(),
        },
        Line::Reasoning { part, .. } => part
            .text
            .filter(|text| !text.is_empty())
            .map(|delta| vec![Event::Reasoning { delta }])
            .unwrap_or_default(),
        Line::ToolUse { part, .. } => tool_events(part),
        Line::Error { error, .. } => {
            let message = error.message();
            state.error = Some(match state.error.take() {
                Some(existing) => format!("{existing}\n{message}"),
                None => message,
            });
            Vec::new()
        }
        Line::Other => Vec::new(),
    }
}
