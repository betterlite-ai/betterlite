use serde::Deserialize;
use serde_json::Value;

use super::error::McpEventError;
use super::event::{EventField, EventItem, ListedTool, McpEventKind, StreamEvent, ToolAnnotations};

#[derive(Debug, Clone, Deserialize)]
pub struct ServerEntry {
    #[serde(default)]
    pub server_label: Value,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ToolAttribute<Content> {
    #[default]
    Absent,
    Value(Content),
    Raises(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredTool {
    pub name: ToolAttribute<Option<String>>,
    pub description: ToolAttribute<Option<String>>,
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
    pub events: Vec<StreamEvent>,
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
            name: read_attribute(&tool.name)?.unwrap_or_else(|| Some("unknown".to_owned())),
            description: read_attribute(&tool.description)?.unwrap_or_else(|| Some(String::new())),
            annotations: ToolAnnotations { read_only: false },
            input_schema: read_attribute(&tool.input_schema)?,
        });
    }
    Ok(formatted)
}

fn status_events(item_id: &str, statuses: &[(McpEventKind, i64)]) -> Vec<StreamEvent> {
    statuses
        .iter()
        .map(|&(kind, sequence_number)| {
            let mut event = StreamEvent::new(kind, sequence_number, 0);
            event
                .fields
                .insert("item_id".to_owned(), EventField::from(item_id));
            event
        })
        .collect()
}

fn push_done_item(
    events: &mut Vec<StreamEvent>,
    item_id: &str,
    server_label: String,
    tools: Vec<ListedTool>,
) {
    let mut done = StreamEvent::new(McpEventKind::OutputItemDone, 1, 0);
    done.fields.insert(
        "item".to_owned(),
        EventField::from(EventItem::ListTools {
            id: item_id.to_owned(),
            server_label,
            tools,
        }),
    );
    events.push(done);
}

fn label_text(label: &Value) -> String {
    match label {
        Value::Null => String::new(),
        Value::Bool(true) => "True".to_owned(),
        Value::Bool(false) => "False".to_owned(),
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Array(_) | Value::Object(_) => label.to_string(),
    }
}

fn failure_outcome(
    mut events: Vec<StreamEvent>,
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
                .map(|server| label_text(&server.server_label))
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
