use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;

use super::error::McpEventError;
use super::event::{EventItem, ListedTool, McpEvent, McpEventKind, ToolAnnotations};

#[derive(Debug, Clone, Deserialize)]
pub struct ServerEntry {
    #[serde(default)]
    pub server_label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolAttribute<Content> {
    Absent,
    Value(Content),
    Raises(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredTool {
    pub name: ToolAttribute<String>,
    pub description: ToolAttribute<String>,
    pub input_schema: ToolAttribute<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticLevel {
    Debug,
    Error,
    Traceback,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub level: DiagnosticLevel,
    pub message: String,
}

#[derive(Debug)]
pub struct ListToolsOutcome {
    pub events: Vec<McpEvent>,
    pub diagnostics: Vec<Diagnostic>,
}

fn read_attribute<Content>(
    attribute: &ToolAttribute<Content>,
) -> Result<Option<Content>, McpEventError>
where
    Content: Clone,
{
    match attribute {
        ToolAttribute::Absent => Ok(None),
        ToolAttribute::Value(content) => Ok(Some(content.clone())),
        ToolAttribute::Raises(message) => Err(McpEventError::ToolAttribute {
            message: message.clone(),
        }),
    }
}

fn format_tools(tools: &[DiscoveredTool]) -> Result<Vec<ListedTool>, McpEventError> {
    let mut formatted = Vec::with_capacity(tools.len());
    for tool in tools {
        formatted.push(ListedTool {
            name: read_attribute(&tool.name)?.unwrap_or_else(|| "unknown".to_owned()),
            description: read_attribute(&tool.description)?.unwrap_or_default(),
            annotations: ToolAnnotations { read_only: false },
            input_schema: read_attribute(&tool.input_schema)?,
        });
    }
    Ok(formatted)
}

fn status_events(item_id: &str, statuses: &[(McpEventKind, i64)]) -> Vec<McpEvent> {
    statuses
        .iter()
        .map(|&(kind, sequence_number)| McpEvent {
            kind,
            sequence_number,
            output_index: 0,
            item_id: Some(item_id.to_owned()),
            delta: None,
            arguments: None,
            item: None,
            extra: BTreeMap::new(),
        })
        .collect()
}

fn push_done_item(
    events: &mut Vec<McpEvent>,
    item_id: &str,
    server_label: String,
    tools: Vec<ListedTool>,
) {
    events.push(McpEvent {
        kind: McpEventKind::OutputItemDone,
        sequence_number: 1,
        output_index: 0,
        item_id: None,
        delta: None,
        arguments: None,
        item: Some(EventItem::ListTools {
            id: item_id.to_owned(),
            server_label,
            tools,
        }),
        extra: BTreeMap::new(),
    });
}

fn failure_outcome(
    mut events: Vec<McpEvent>,
    item_id: &str,
    error: &McpEventError,
) -> ListToolsOutcome {
    events.extend(status_events(
        item_id,
        &[(McpEventKind::ListToolsFailed, 2)],
    ));
    push_done_item(&mut events, item_id, String::new(), Vec::new());
    ListToolsOutcome {
        events,
        diagnostics: vec![
            Diagnostic {
                level: DiagnosticLevel::Error,
                message: format!("Error creating MCP list tools events: {error}"),
            },
            Diagnostic {
                level: DiagnosticLevel::Traceback,
                message: error.to_string(),
            },
        ],
    }
}

#[must_use]
pub fn create_mcp_list_tools_events(
    servers: &[ServerEntry],
    base_item_id: &str,
    tools: &[DiscoveredTool],
) -> ListToolsOutcome {
    let mut events = status_events(
        base_item_id,
        &[
            (McpEventKind::ListToolsInProgress, 1),
            (McpEventKind::ListToolsCompleted, 2),
        ],
    );
    match format_tools(tools) {
        Ok(formatted) => {
            let server_label = servers
                .first()
                .and_then(|server| server.server_label.clone())
                .unwrap_or_default();
            push_done_item(&mut events, base_item_id, server_label, formatted);
            let message = format!("Created {} MCP discovery events", events.len());
            ListToolsOutcome {
                events,
                diagnostics: vec![Diagnostic {
                    level: DiagnosticLevel::Debug,
                    message,
                }],
            }
        }
        Err(error) => failure_outcome(events, base_item_id, &error),
    }
}
