#![expect(
    clippy::disallowed_types,
    reason = "codex's mcp tool arguments/result/error and the ToolCall.input boundary carry open JSON with no fixed schema"
)]
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

#[path = "tool_projection.rs"]
mod tool_projection;

pub(crate) use tool_projection::tool_projection;
