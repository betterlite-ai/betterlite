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
    let native = part
        .tool
        .as_ref()
        .filter(|tool| !tool.is_null())
        .map(super::wire::value_to_string)
        .unwrap_or_default();
    let call_id = part
        .call_id
        .as_ref()
        .filter(|id| id.as_str().is_some_and(|value| !value.is_empty()))
        .map(super::wire::value_to_string)
        .or_else(|| {
            part.id
                .as_ref()
                .filter(|id| id.as_str().is_some_and(|value| !value.is_empty()))
                .map(super::wire::value_to_string)
        })
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
    vec![call, tool_result(part, &native, call_id)]
}

fn tool_result(part: ToolPart, native: &str, call_id: String) -> Event {
    let (output, is_error) = if part
        .state
        .status
        .as_ref()
        .is_some_and(|status| status.as_str() == Some("error"))
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
    if let Some(error) = state
        .error
        .as_ref()
        .filter(|error| super::wire::is_truthy(error))
    {
        return super::wire::value_to_string(error);
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
            let Some(delta) = part
                .text
                .filter(|text| !text.is_null())
                .map(|text| super::wire::value_to_string(&text))
                .filter(|text| !text.is_empty())
            else {
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
            state.error = Some(format!("opencode line could not be parsed: {line}"));
            Vec::new()
        }
    }
}

fn text_event(part: super::wire::TextPart, state: &mut OpenCodeStreamState) -> Vec<Event> {
    let Some(text) = part
        .text
        .filter(|text| !text.is_null())
        .map(|text| super::wire::value_to_string(&text))
        .filter(|text| !text.is_empty())
    else {
        return Vec::new();
    };
    state.step_texts.push(text.clone());
    state.final_text = state.step_texts.join("\n\n");
    vec![Event::Text { delta: text }]
}
