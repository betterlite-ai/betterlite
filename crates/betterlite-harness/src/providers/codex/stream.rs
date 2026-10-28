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
    line.get("type").and_then(Value::as_str)
}

pub(super) fn thread_id(line: &JsonObject) -> Option<&str> {
    line.get("thread_id")
        .and_then(Value::as_str)
        .filter(|candidate| !candidate.is_empty())
}

pub(super) fn error_message(line: &JsonObject) -> Option<&str> {
    line.get("message").and_then(Value::as_str)
}

pub(super) fn turn_failed_message(line: &JsonObject) -> Option<&str> {
    line.get("error")
        .and_then(Value::as_object)
        .and_then(|detail| detail.get("message"))
        .and_then(Value::as_str)
        .or_else(|| line.get("error").and_then(Value::as_str))
}

pub(super) fn object(value: Option<&Value>) -> Option<JsonObject> {
    value.and_then(Value::as_object).map(|entries| {
        entries
            .iter()
            .map(|(key, entry)| (key.clone(), entry.clone()))
            .collect()
    })
}

pub(super) fn item_kind(item: &JsonObject) -> &str {
    item.get("type").and_then(Value::as_str).unwrap_or_default()
}

pub(super) fn item_id(item: &JsonObject) -> String {
    item.get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

pub(super) fn item_text(item: &JsonObject) -> Option<&str> {
    item.get("text").and_then(Value::as_str)
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
    match item_kind(item) {
        "command_execution" => command_execution_output(item, status),
        "file_change" => file_change_output(item, status),
        "web_search" => (String::new(), status == Some("failed")),
        _ => mcp_tool_output(item, status),
    }
}

fn command_execution_output(item: &JsonObject, status: Option<&str>) -> (String, bool) {
    let failed = status == Some("failed")
        || item
            .get("exit_code")
            .is_some_and(|value| value.as_i64().is_some_and(|code| code != 0));
    (
        item.get("aggregated_output")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        failed,
    )
}

fn file_change_output(item: &JsonObject, status: Option<&str>) -> (String, bool) {
    let summary = item
        .get("changes")
        .and_then(Value::as_array)
        .map(|changes| {
            changes
                .iter()
                .map(change_summary)
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    (summary, status == Some("failed"))
}

fn change_summary(change: &Value) -> String {
    format!(
        "{} {}",
        change
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        change
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or_default()
    )
    .trim()
    .to_owned()
}

fn mcp_tool_output(item: &JsonObject, status: Option<&str>) -> (String, bool) {
    item.get("error")
        .filter(|error| !error.is_null())
        .map_or_else(
            || {
                (
                    item.get("result")
                        .filter(|value| !value.is_null())
                        .map_or_else(String::new, |value| {
                            value
                                .as_str()
                                .map_or_else(|| value.to_string(), str::to_owned)
                        }),
                    status == Some("failed"),
                )
            },
            |error| {
                (
                    error
                        .get("message")
                        .and_then(Value::as_str)
                        .or_else(|| error.as_str())
                        .map_or_else(|| error.to_string(), str::to_owned),
                    true,
                )
            },
        )
}
