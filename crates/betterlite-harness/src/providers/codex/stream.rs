#![expect(
    clippy::disallowed_types,
    reason = "Codex emits untyped JSONL events; this module is the one place that reads them field by field"
)]
use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::types::JsonObject;

#[derive(Clone, Debug, Default)]
pub struct CodexStreamState {
    pub thread_id: Option<String>,
    pub final_text: String,
    pub error: Option<String>,
    pub failed: bool,
    pub started: BTreeSet<String>,
}

pub(super) fn event_type(line: &JsonObject) -> Option<&str> {
    let kind = line.get("type");
    kind.and_then(Value::as_str)
}

pub(super) fn thread_id(line: &JsonObject) -> Option<&str> {
    let candidate = line.get("thread_id").and_then(Value::as_str);
    candidate.filter(|value| !value.is_empty())
}

pub(super) fn error_message(line: &JsonObject) -> Option<&str> {
    let message = line.get("message");
    message.and_then(Value::as_str)
}

pub(super) fn turn_failed_message(line: &JsonObject) -> Option<&str> {
    let detail_message = line
        .get("error")
        .and_then(Value::as_object)
        .and_then(|detail| detail.get("message"))
        .and_then(Value::as_str);
    detail_message.or_else(|| line.get("error").and_then(Value::as_str))
}

pub(super) fn object(value: Option<&Value>) -> Option<JsonObject> {
    let entries = value.and_then(Value::as_object)?;
    Some(
        entries
            .iter()
            .map(|(key, entry)| (key.clone(), entry.clone()))
            .collect(),
    )
}

pub(super) fn item_kind(item: &JsonObject) -> &str {
    let kind = item.get("type").and_then(Value::as_str);
    kind.unwrap_or_default()
}

pub(super) fn item_id(item: &JsonObject) -> String {
    let id = item.get("id").and_then(Value::as_str).unwrap_or_default();
    id.to_owned()
}

pub(super) fn item_text(item: &JsonObject) -> Option<&str> {
    let text = item.get("text");
    text.and_then(Value::as_str)
}

pub(super) fn tool_input(item: &JsonObject) -> (String, String, JsonObject, bool) {
    match item_kind(item) {
        "command_execution" => (
            "bash".to_owned(),
            "command_execution".to_owned(),
            BTreeMap::from([(
                "command".to_owned(),
                item.get("command")
                    .cloned()
                    .unwrap_or(Value::String(String::new())),
            )]),
            true,
        ),
        "file_change" => (
            "edit".to_owned(),
            "apply_patch".to_owned(),
            BTreeMap::from([(
                "changes".to_owned(),
                item.get("changes")
                    .cloned()
                    .unwrap_or(Value::Array(Vec::new())),
            )]),
            true,
        ),
        "web_search" => (
            "web_search".to_owned(),
            "web_search".to_owned(),
            BTreeMap::from([(
                "query".to_owned(),
                item.get("query")
                    .cloned()
                    .unwrap_or(Value::String(String::new())),
            )]),
            true,
        ),
        _ => mcp_tool_input(item),
    }
}

fn mcp_tool_input(item: &JsonObject) -> (String, String, JsonObject, bool) {
    let server = item
        .get("server")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let native = item.get("tool").and_then(Value::as_str).unwrap_or_default();
    let input = match item.get("arguments") {
        Some(Value::Object(map)) => map
            .iter()
            .map(|(key, entry)| (key.clone(), entry.clone()))
            .collect(),
        Some(value) => BTreeMap::from([("arguments".to_owned(), value.clone())]),
        None => BTreeMap::from([("arguments".to_owned(), Value::Null)]),
    };
    let name = if server.is_empty() {
        native.to_owned()
    } else {
        format!("{server}.{native}")
    };
    (name, native.to_owned(), input, false)
}

pub(super) fn tool_output(item: &JsonObject) -> (String, bool) {
    let status = item.get("status").and_then(Value::as_str);
    let failed = status == Some("failed");
    match item_kind(item) {
        "command_execution" => (aggregated_output_text(item), failed || exited_nonzero(item)),
        "file_change" => (file_change_summary(item), failed),
        "web_search" => (String::new(), failed),
        _ => mcp_tool_result_text(item, failed),
    }
}

fn mcp_tool_result_text(item: &JsonObject, failed: bool) -> (String, bool) {
    let Some(error) = item.get("error").filter(|error| !error.is_null()) else {
        return (mcp_tool_result_value(item), failed);
    };
    (mcp_tool_error_message(error), true)
}

fn mcp_tool_result_value(item: &JsonObject) -> String {
    let Some(value) = item.get("result").filter(|value| !value.is_null()) else {
        return String::new();
    };
    value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_owned)
}

fn mcp_tool_error_message(error: &Value) -> String {
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| error.as_str());
    message.map_or_else(|| error.to_string(), str::to_owned)
}

fn aggregated_output_text(item: &JsonObject) -> String {
    let text = item.get("aggregated_output").and_then(Value::as_str);
    text.unwrap_or_default().to_owned()
}

fn exited_nonzero(item: &JsonObject) -> bool {
    let exit_code = item.get("exit_code");
    exit_code.is_some_and(|value| value.as_i64().is_some_and(|code| code != 0))
}

fn file_change_summary(item: &JsonObject) -> String {
    let Some(changes) = item.get("changes").and_then(Value::as_array) else {
        return String::new();
    };
    changes
        .iter()
        .map(change_summary)
        .collect::<Vec<_>>()
        .join("\n")
}

fn change_summary(change: &Value) -> String {
    let kind = change
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let path = change
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or_default();
    format!("{kind} {path}").trim().to_owned()
}
