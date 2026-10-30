use serde_json::Value;

use crate::events::Event;
use crate::payloads::python_json::python_json_text;
use crate::types::JsonObject;

#[derive(Clone, Debug, Default)]
pub struct OpenCodeStreamState {
    pub session_id: Option<String>,
    pub final_text: String,
    pub error: Option<String>,
    pub step_texts: Vec<String>,
}

pub(super) fn truthy_string(object: &JsonObject, key: &str) -> Option<String> {
    match object.get(key) {
        Some(Value::String(text)) if !text.is_empty() => Some(text.clone()),
        _ => None,
    }
}

fn as_object(object: &JsonObject, key: &str) -> JsonObject {
    match object.get(key) {
        Some(Value::Object(entries)) => entries
            .iter()
            .map(|(field, value)| (field.clone(), value.clone()))
            .collect(),
        _ => JsonObject::new(),
    }
}

fn json_dumps_like(object: &JsonObject, key: &str) -> String {
    let value = object.get(key).cloned().unwrap_or(Value::Null);
    python_json_text(&value).unwrap_or_else(|_| value.to_string())
}

// opencode gates write, edit and apply_patch with the single `edit` permission (NATIVE_TO_NORMALIZED)
fn normalized_name(native: &str) -> String {
    match native {
        "write" => "write".to_owned(),
        "edit" | "multiedit" | "patch" | "apply_patch" => "edit".to_owned(),
        "bash" => "bash".to_owned(),
        "glob" => "glob".to_owned(),
        "grep" => "grep".to_owned(),
        "list" => "ls".to_owned(),
        "webfetch" | "websearch" => "web_search".to_owned(),
        other => other.to_owned(),
    }
}

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

fn error_message(line: &JsonObject) -> String {
    match line.get("error") {
        Some(Value::Object(error_entries)) => {
            let error_fields: JsonObject = error_entries
                .iter()
                .map(|(field, value)| (field.clone(), value.clone()))
                .collect();
            let data = as_object(&error_fields, "data");
            truthy_string(&data, "message")
                .or_else(|| truthy_string(&error_fields, "name"))
                .unwrap_or_else(|| "opencode reported an error".to_owned())
        }
        Some(Value::String(text)) if !text.is_empty() => text.clone(),
        _ => "opencode reported an error".to_owned(),
    }
}

fn tool_input(state: &JsonObject) -> JsonObject {
    match state.get("input") {
        Some(Value::Object(entries)) => entries
            .iter()
            .map(|(field, value)| (field.clone(), value.clone()))
            .collect(),
        Some(other) => JsonObject::from([("input".to_owned(), other.clone())]),
        None => JsonObject::new(),
    }
}

fn tool_events(part: &JsonObject) -> Vec<Event> {
    let native = truthy_string(part, "tool").unwrap_or_default();
    let call_id = truthy_string(part, "callID")
        .or_else(|| truthy_string(part, "id"))
        .unwrap_or_default();
    let state = as_object(part, "state");
    let call = Event::ToolCall {
        id: call_id.clone(),
        name: normalized_name(&native),
        native_name: native.clone(),
        input: tool_input(&state),
        builtin: BUILTIN_TOOLS.contains(&native.as_str()),
    };
    let status = state.get("status").and_then(Value::as_str);
    if status == Some("error") {
        let message = truthy_string(&state, "error")
            .or_else(|| truthy_string(&state, "output"))
            .unwrap_or_else(|| "tool failed".to_owned());
        return vec![
            call,
            Event::ToolResult {
                id: call_id,
                output: message,
                is_error: true,
            },
        ];
    }
    let output = match state.get("output") {
        Some(Value::String(text)) => text.clone(),
        _ => json_dumps_like(&state, "output"),
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

// step_finish token counts are ignored on purpose: the session endpoint accounts usage
pub(super) fn transform_line(line: &JsonObject, state: &mut OpenCodeStreamState) -> Vec<Event> {
    if state.session_id.is_none() {
        state.session_id = truthy_string(line, "sessionID");
    }
    let event_type = line.get("type").and_then(Value::as_str);
    let part = as_object(line, "part");
    match event_type {
        Some("step_start") => {
            state.step_texts.clear();
            Vec::new()
        }
        Some("text") => {
            let Some(text) = truthy_string(&part, "text") else {
                return Vec::new();
            };
            state.step_texts.push(text.clone());
            state.final_text = state.step_texts.join("\n\n");
            vec![Event::Text { delta: text }]
        }
        Some("reasoning") => truthy_string(&part, "text")
            .map(|text| vec![Event::Reasoning { delta: text }])
            .unwrap_or_default(),
        Some("tool_use") => tool_events(&part),
        Some("error") => {
            let message = error_message(line);
            state.error = Some(match state.error.take() {
                Some(existing) => format!("{existing}\n{message}"),
                None => message,
            });
            Vec::new()
        }
        _ => Vec::new(),
    }
}
