#![expect(
    clippy::disallowed_types,
    reason = "codex's mcp tool arguments/result/error and the ToolCall.input boundary carry open JSON with no fixed schema"
)]
use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

use crate::types::JsonObject;

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum Item {
    #[serde(rename = "agent_message")]
    AgentMessage {
        #[serde(default)]
        id: String,
        #[serde(default)]
        text: Option<String>,
    },
    #[serde(rename = "reasoning")]
    Reasoning {
        #[serde(default)]
        id: String,
        #[serde(default)]
        text: Option<String>,
    },
    #[serde(rename = "command_execution")]
    CommandExecution {
        #[serde(default)]
        id: String,
        #[serde(default)]
        command: Option<String>,
        #[serde(default)]
        aggregated_output: Option<String>,
        #[serde(default)]
        exit_code: Option<i64>,
        #[serde(default)]
        status: Option<String>,
    },
    #[serde(rename = "file_change")]
    FileChange {
        #[serde(default)]
        id: String,
        #[serde(default)]
        changes: Option<Vec<Value>>,
        #[serde(default)]
        status: Option<String>,
    },
    #[serde(rename = "web_search")]
    WebSearch {
        #[serde(default)]
        id: String,
        #[serde(default)]
        query: Option<String>,
        #[serde(default)]
        status: Option<String>,
    },
    #[serde(rename = "mcp_tool_call")]
    McpToolCall {
        #[serde(default)]
        id: String,
        #[serde(default)]
        server: Option<String>,
        #[serde(default)]
        tool: Option<String>,
        #[serde(default)]
        arguments: Option<Value>,
        #[serde(default)]
        result: Option<Value>,
        #[serde(default)]
        error: Option<Value>,
        #[serde(default)]
        status: Option<String>,
    },
    #[serde(other)]
    Other,
}

pub(crate) fn parse(line: &JsonObject) -> Option<super::StreamEvent> {
    let object = line
        .iter()
        .map(|(key, entry)| (key.clone(), entry.clone()))
        .collect();
    serde_json::from_value(Value::Object(object)).ok()
}

pub(crate) fn tool_input(item: &Item) -> (String, String, JsonObject, bool) {
    match item {
        Item::CommandExecution { command, .. } => (
            "bash".to_owned(),
            "command_execution".to_owned(),
            BTreeMap::from([(
                "command".to_owned(),
                Value::String(command.clone().unwrap_or_default()),
            )]),
            true,
        ),
        Item::FileChange { changes, .. } => (
            "edit".to_owned(),
            "apply_patch".to_owned(),
            BTreeMap::from([(
                "changes".to_owned(),
                Value::Array(changes.clone().unwrap_or_default()),
            )]),
            true,
        ),
        Item::WebSearch { query, .. } => (
            "web_search".to_owned(),
            "web_search".to_owned(),
            BTreeMap::from([(
                "query".to_owned(),
                Value::String(query.clone().unwrap_or_default()),
            )]),
            true,
        ),
        Item::McpToolCall {
            server,
            tool,
            arguments,
            ..
        } => mcp_tool_input(server.as_deref(), tool.as_deref(), arguments.as_ref()),
        Item::AgentMessage { .. } | Item::Reasoning { .. } | Item::Other => {
            (String::new(), String::new(), JsonObject::new(), false)
        }
    }
}

fn mcp_tool_input(
    server: Option<&str>,
    tool: Option<&str>,
    arguments: Option<&Value>,
) -> (String, String, JsonObject, bool) {
    let native = tool.unwrap_or_default().to_owned();
    let input = match arguments {
        Some(Value::Object(map)) => map
            .iter()
            .map(|(key, entry)| (key.clone(), entry.clone()))
            .collect(),
        Some(value) => BTreeMap::from([("arguments".to_owned(), value.clone())]),
        None => BTreeMap::from([("arguments".to_owned(), Value::Null)]),
    };
    let name = server
        .filter(|value| !value.is_empty())
        .map_or_else(|| native.clone(), |prefix| format!("{prefix}.{native}"));
    (name, native, input, false)
}

pub(crate) fn tool_output(item: &Item) -> (String, bool) {
    match item {
        Item::CommandExecution {
            aggregated_output,
            exit_code,
            status,
            ..
        } => {
            let failed = status.as_deref() == Some("failed");
            let exited_nonzero = exit_code.is_some_and(|code| code != 0);
            (
                aggregated_output.clone().unwrap_or_default(),
                failed || exited_nonzero,
            )
        }
        Item::FileChange {
            changes, status, ..
        } => (
            change_summary(changes.as_deref()),
            status.as_deref() == Some("failed"),
        ),
        Item::WebSearch { status, .. } => (String::new(), status.as_deref() == Some("failed")),
        Item::McpToolCall {
            result,
            error,
            status,
            ..
        } => {
            let failed = status.as_deref() == Some("failed");
            error.as_ref().filter(|value| is_truthy(value)).map_or_else(
                || (mcp_result_text(result.as_ref()), failed),
                |reported| (mcp_error_message(reported), true),
            )
        }
        Item::AgentMessage { .. } | Item::Reasoning { .. } | Item::Other => (String::new(), false),
    }
}

fn change_summary(changes: Option<&[Value]>) -> String {
    changes
        .unwrap_or_default()
        .iter()
        .map(|change| {
            let kind = change
                .get("kind")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let path = change
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or_default();
            format!("{kind} {path}").trim().to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn mcp_result_text(result: Option<&Value>) -> String {
    let Some(value) = result.filter(|value| !value.is_null()) else {
        return String::new();
    };
    value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_owned)
}

fn mcp_error_message(error: &Value) -> String {
    match error {
        Value::Object(fields) => fields
            .get("message")
            .and_then(Value::as_str)
            .map_or_else(|| "None".to_owned(), str::to_owned),
        Value::String(text) => text.clone(),
        other @ (Value::Null | Value::Bool(_) | Value::Number(_) | Value::Array(_)) => {
            other.to_string()
        }
    }
}

fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_none_or(|parsed| parsed != 0.0_f64),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(fields) => !fields.is_empty(),
    }
}
