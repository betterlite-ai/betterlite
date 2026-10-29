#![expect(
    clippy::disallowed_types,
    reason = "codex's mcp tool arguments/result/error and the ToolCall.input boundary carry open JSON with no fixed schema"
)]
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::payloads::python_json::OrderedJson;
use crate::types::JsonObject;

use super::ScalarJson;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub(crate) struct FileChangeEntry {
    #[serde(default)]
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) path: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
pub(crate) enum Item {
    #[serde(rename = "agent_message")]
    AgentMessage {
        #[serde(default)]
        id: Option<ScalarJson>,
        #[serde(default)]
        text: Option<String>,
    },
    #[serde(rename = "reasoning")]
    Reasoning {
        #[serde(default)]
        id: Option<ScalarJson>,
        #[serde(default)]
        text: Option<String>,
    },
    #[serde(rename = "command_execution")]
    CommandExecution {
        #[serde(default)]
        id: Option<ScalarJson>,
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
        id: Option<ScalarJson>,
        #[serde(default)]
        changes: Option<Vec<FileChangeEntry>>,
        #[serde(default)]
        status: Option<String>,
    },
    #[serde(rename = "web_search")]
    WebSearch {
        #[serde(default)]
        id: Option<ScalarJson>,
        #[serde(default)]
        query: Option<String>,
        #[serde(default)]
        status: Option<String>,
    },
    #[serde(rename = "mcp_tool_call")]
    McpToolCall(McpToolCallItem),
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
pub(crate) struct McpToolCallItem {
    #[serde(default)]
    pub(crate) id: Option<ScalarJson>,
    #[serde(default)]
    pub(crate) server: Option<String>,
    #[serde(default)]
    pub(crate) tool: Option<String>,
    #[serde(default)]
    pub(crate) arguments: Option<Value>,
    #[serde(default)]
    pub(crate) result: Option<OrderedJson>,
    #[serde(default)]
    pub(crate) error: Option<Value>,
    #[serde(default)]
    pub(crate) status: Option<String>,
}

pub(crate) fn parse(line: &JsonObject) -> Option<super::StreamEvent> {
    let object: serde_json::Map<String, Value> = line
        .iter()
        .map(|(key, entry)| (key.clone(), entry.clone()))
        .collect();
    let value = Value::Object(object);
    serde_json::from_value(value).ok().or_else(|| {
        (line.get("type").and_then(Value::as_str) == Some("turn.failed"))
            .then(|| super::StreamEvent::TurnFailed { error: None })
    })
}

#[path = "tool_projection.rs"]
mod tool_projection;

pub(crate) use tool_projection::tool_projection;
