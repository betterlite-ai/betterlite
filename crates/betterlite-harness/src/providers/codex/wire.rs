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

pub(crate) struct ToolProjection {
    pub(crate) name: String,
    pub(crate) native_name: String,
    pub(crate) input: JsonObject,
    pub(crate) builtin: bool,
    pub(crate) output: String,
    pub(crate) is_error: bool,
}

pub(crate) fn tool_projection(item: &Item) -> ToolProjection {
    match item {
        Item::CommandExecution {
            command,
            aggregated_output,
            exit_code,
            status,
            ..
        } => command_execution_projection(
            command.as_deref(),
            aggregated_output.as_deref(),
            *exit_code,
            status.as_deref(),
        ),
        Item::FileChange {
            changes, status, ..
        } => file_change_projection(changes.as_deref(), status.as_deref()),
        Item::WebSearch { query, status, .. } => {
            web_search_projection(query.as_deref(), status.as_deref())
        }
        Item::McpToolCall {
            server,
            tool,
            arguments,
            result,
            error,
            status,
            ..
        } => mcp_tool_projection(
            (server.as_deref(), tool.as_deref(), arguments.as_ref()),
            (result.as_ref(), error.as_ref(), status.as_deref()),
        ),
        Item::AgentMessage { .. } | Item::Reasoning { .. } | Item::Other => ToolProjection {
            name: String::new(),
            native_name: String::new(),
            input: JsonObject::new(),
            builtin: false,
            output: String::new(),
            is_error: false,
        },
    }
}

fn command_execution_projection(
    command: Option<&str>,
    aggregated_output: Option<&str>,
    exit_code: Option<i64>,
    status: Option<&str>,
) -> ToolProjection {
    let failed = status == Some("failed");
    let exited_nonzero = exit_code.is_some_and(|code| code != 0);
    ToolProjection {
        name: "bash".to_owned(),
        native_name: "command_execution".to_owned(),
        input: BTreeMap::from([(
            "command".to_owned(),
            Value::String(command.unwrap_or_default().to_owned()),
        )]),
        builtin: true,
        output: aggregated_output.unwrap_or_default().to_owned(),
        is_error: failed || exited_nonzero,
    }
}

fn file_change_projection(changes: Option<&[Value]>, status: Option<&str>) -> ToolProjection {
    let output = file_change_summary(changes);
    ToolProjection {
        name: "edit".to_owned(),
        native_name: "apply_patch".to_owned(),
        input: BTreeMap::from([(
            "changes".to_owned(),
            Value::Array(changes.unwrap_or_default().to_vec()),
        )]),
        builtin: true,
        output,
        is_error: status == Some("failed"),
    }
}

fn web_search_projection(query: Option<&str>, status: Option<&str>) -> ToolProjection {
    let input = BTreeMap::from([(
        "query".to_owned(),
        Value::String(query.unwrap_or_default().to_owned()),
    )]);
    ToolProjection {
        name: "web_search".to_owned(),
        native_name: "web_search".to_owned(),
        input,
        builtin: true,
        output: String::new(),
        is_error: status == Some("failed"),
    }
}

fn mcp_tool_projection(
    call: (Option<&str>, Option<&str>, Option<&Value>),
    outcome: (Option<&Value>, Option<&Value>, Option<&str>),
) -> ToolProjection {
    let (server, tool, arguments) = call;
    let (result, error, status) = outcome;
    let (name, native_name, input) = mcp_tool_input(server, tool, arguments);
    let (output, is_error) = mcp_tool_output(result, error, status);
    ToolProjection {
        name,
        native_name,
        input,
        builtin: false,
        output,
        is_error,
    }
}

fn mcp_tool_input(
    server: Option<&str>,
    tool: Option<&str>,
    arguments: Option<&Value>,
) -> (String, String, JsonObject) {
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
    (name, native, input)
}

fn mcp_tool_output(
    result: Option<&Value>,
    error: Option<&Value>,
    status: Option<&str>,
) -> (String, bool) {
    let failed = status == Some("failed");
    error.filter(|value| is_truthy(value)).map_or_else(
        || (mcp_result_text(result), failed),
        |reported| (mcp_error_message(reported), true),
    )
}

fn file_change_summary(changes: Option<&[Value]>) -> String {
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
