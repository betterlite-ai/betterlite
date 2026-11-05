use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::error::McpEventError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum McpEventKind {
    #[serde(rename = "response.mcp_list_tools.in_progress")]
    ListToolsInProgress,
    #[serde(rename = "response.mcp_list_tools.completed")]
    ListToolsCompleted,
    #[serde(rename = "response.mcp_list_tools.failed")]
    ListToolsFailed,
    #[serde(rename = "response.mcp_call.in_progress")]
    CallInProgress,
    #[serde(rename = "response.mcp_call_arguments.delta")]
    CallArgumentsDelta,
    #[serde(rename = "response.mcp_call_arguments.done")]
    CallArgumentsDone,
    #[serde(rename = "response.mcp_call.completed")]
    CallCompleted,
    #[serde(rename = "response.mcp_call.failed")]
    CallFailed,
    #[serde(rename = "response.output_item.done")]
    OutputItemDone,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolAnnotations {
    pub read_only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListedTool {
    pub name: String,
    pub description: String,
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
pub struct McpEvent {
    #[serde(rename = "type")]
    pub kind: McpEventKind,
    pub sequence_number: i64,
    pub output_index: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<EventItem>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

pub fn set_event_field(
    event: &mut McpEvent,
    name: &str,
    input: Value,
) -> Result<(), McpEventError> {
    let wrong_type = |expected| McpEventError::FieldType {
        field: name.to_owned(),
        expected,
    };
    match (name, input) {
        ("type", kind) => {
            event.kind = serde_json::from_value(kind)
                .map_err(|source| McpEventError::EventType { source })?;
        }
        ("sequence_number", number) => {
            event.sequence_number = number.as_i64().ok_or_else(|| wrong_type("an integer"))?;
        }
        ("output_index", number) => {
            event.output_index = number.as_i64().ok_or_else(|| wrong_type("an integer"))?;
        }
        ("item_id", Value::String(text)) => event.item_id = Some(text),
        ("delta", Value::String(text)) => event.delta = Some(text),
        ("arguments", Value::String(text)) => event.arguments = Some(text),
        ("item_id" | "delta" | "arguments", _) => return Err(wrong_type("a string")),
        ("item", item) => {
            event.item = Some(
                serde_json::from_value(item)
                    .map_err(|source| McpEventError::ItemShape { source })?,
            );
        }
        (_, other) => {
            event.extra.insert(name.to_owned(), other);
        }
    }
    Ok(())
}
