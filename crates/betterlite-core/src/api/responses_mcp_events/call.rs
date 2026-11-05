use std::collections::BTreeMap;

use serde::Deserialize;

use super::error::McpEventError;
use super::event::{EventItem, McpEvent, McpEventKind};

#[derive(Debug, Clone, Deserialize)]
pub struct McpCallRequest {
    pub tool_name: String,
    pub arguments: String,
    #[serde(default)]
    pub result: Option<String>,
    #[serde(default)]
    pub base_item_id: Option<String>,
    #[serde(default = "default_sequence_start")]
    pub sequence_start: i64,
    #[serde(default)]
    pub output_index: i64,
}

fn default_sequence_start() -> i64 {
    1
}

fn sequence_at(request: &McpCallRequest, offset: i64) -> Result<i64, McpEventError> {
    request
        .sequence_start
        .checked_add(offset)
        .ok_or(McpEventError::SequenceOverflow {
            start: request.sequence_start,
            offset,
        })
}

fn argument_events(
    request: &McpCallRequest,
    item_id: &str,
) -> Result<Vec<McpEvent>, McpEventError> {
    Ok(vec![
        McpEvent {
            kind: McpEventKind::CallInProgress,
            sequence_number: sequence_at(request, 0)?,
            output_index: request.output_index,
            item_id: Some(item_id.to_owned()),
            delta: None,
            arguments: None,
            item: None,
            extra: BTreeMap::new(),
        },
        McpEvent {
            kind: McpEventKind::CallArgumentsDelta,
            sequence_number: sequence_at(request, 1)?,
            output_index: request.output_index,
            item_id: Some(item_id.to_owned()),
            delta: Some(request.arguments.clone()),
            arguments: None,
            item: None,
            extra: BTreeMap::new(),
        },
        McpEvent {
            kind: McpEventKind::CallArgumentsDone,
            sequence_number: sequence_at(request, 2)?,
            output_index: request.output_index,
            item_id: Some(item_id.to_owned()),
            delta: None,
            arguments: Some(request.arguments.clone()),
            item: None,
            extra: BTreeMap::new(),
        },
    ])
}

fn outcome_events<Id>(
    request: &McpCallRequest,
    item_id: &str,
    mut next_id: Id,
) -> Result<Vec<McpEvent>, McpEventError>
where
    Id: FnMut() -> String,
{
    let sequence_number = sequence_at(request, 3)?;
    let Some(output) = &request.result else {
        return Ok(vec![McpEvent {
            kind: McpEventKind::CallFailed,
            sequence_number,
            output_index: request.output_index,
            item_id: Some(item_id.to_owned()),
            delta: None,
            arguments: None,
            item: None,
            extra: BTreeMap::new(),
        }]);
    };
    Ok(vec![
        McpEvent {
            kind: McpEventKind::CallCompleted,
            sequence_number,
            output_index: request.output_index,
            item_id: Some(item_id.to_owned()),
            delta: None,
            arguments: None,
            item: None,
            extra: BTreeMap::new(),
        },
        McpEvent {
            kind: McpEventKind::OutputItemDone,
            sequence_number: 1,
            output_index: request.output_index,
            item_id: None,
            delta: None,
            arguments: None,
            item: Some(EventItem::Call {
                id: item_id.to_owned(),
                approval_request_id: format!("mcpr_{}", next_id()),
                arguments: request.arguments.clone(),
                error: None,
                name: request.tool_name.clone(),
                output: output.clone(),
                server_label: "litellm".to_owned(),
            }),
            extra: BTreeMap::new(),
        },
    ])
}

pub fn create_mcp_call_events<Id>(
    request: &McpCallRequest,
    mut next_id: Id,
) -> Result<Vec<McpEvent>, McpEventError>
where
    Id: FnMut() -> String,
{
    let item_id = match request.base_item_id.as_deref() {
        Some(supplied) if !supplied.is_empty() => supplied.to_owned(),
        _ => format!("mcp_{}", next_id()),
    };
    let mut events = argument_events(request, &item_id)?;
    events.extend(outcome_events(request, &item_id, next_id)?);
    Ok(events)
}
