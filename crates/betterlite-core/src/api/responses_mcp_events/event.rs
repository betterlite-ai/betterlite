use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpEventKind {
    ListToolsInProgress,
    ListToolsCompleted,
    ListToolsFailed,
    CallInProgress,
    CallArgumentsDelta,
    CallArgumentsDone,
    CallCompleted,
    CallFailed,
    OutputItemDone,
}

impl McpEventKind {
    #[must_use]
    pub const fn wire_name(self) -> &'static str {
        match self {
            Self::ListToolsInProgress => "response.mcp_list_tools.in_progress",
            Self::ListToolsCompleted => "response.mcp_list_tools.completed",
            Self::ListToolsFailed => "response.mcp_list_tools.failed",
            Self::CallInProgress => "response.mcp_call.in_progress",
            Self::CallArgumentsDelta => "response.mcp_call_arguments.delta",
            Self::CallArgumentsDone => "response.mcp_call_arguments.done",
            Self::CallCompleted => "response.mcp_call.completed",
            Self::CallFailed => "response.mcp_call.failed",
            Self::OutputItemDone => "response.output_item.done",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolAnnotations {
    pub read_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListedTool {
    pub name: Option<String>,
    pub description: Option<String>,
    pub annotations: ToolAnnotations,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum EventItem {
    #[serde(rename = "mcp_list_tools")]
    ListTools {
        id: String,
        server_label: String,
        tools: Vec<ListedTool>,
    },
    #[serde(rename = "mcp_call")]
    Call {
        id: String,
        approval_request_id: String,
        arguments: String,
        error: Option<String>,
        name: String,
        output: String,
        server_label: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum EventField {
    Item(EventItem),
    Json(Value),
}

impl From<&str> for EventField {
    fn from(text: &str) -> Self {
        Self::Json(Value::String(text.to_owned()))
    }
}

impl From<i64> for EventField {
    fn from(number: i64) -> Self {
        Self::Json(Value::from(number))
    }
}

impl From<EventItem> for EventField {
    fn from(item: EventItem) -> Self {
        Self::Item(item)
    }
}

// LiteLLM events allow extra fields, so setattr stores any value under any name, as an insert here does
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct StreamEvent {
    pub fields: BTreeMap<String, EventField>,
}

impl StreamEvent {
    #[must_use]
    pub fn new(kind: McpEventKind, sequence_number: i64, output_index: i64) -> Self {
        Self {
            fields: BTreeMap::from([
                ("type".to_owned(), EventField::from(kind.wire_name())),
                (
                    "sequence_number".to_owned(),
                    EventField::from(sequence_number),
                ),
                ("output_index".to_owned(), EventField::from(output_index)),
            ]),
        }
    }
}
