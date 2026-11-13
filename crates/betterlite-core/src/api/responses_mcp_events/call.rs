use serde::Deserialize;

use super::error::McpEventError;
use super::event::{EventField, EventItem, McpEventKind, StreamEvent};

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

const fn default_sequence_start() -> i64 {
    1
}

fn call_event(
    request: &McpCallRequest,
    item_id: &str,
    kind: McpEventKind,
    offset: i64,
) -> Result<StreamEvent, McpEventError> {
    let sequence_number =
        request
            .sequence_start
            .checked_add(offset)
            .ok_or(McpEventError::SequenceOverflow {
                start: request.sequence_start,
                offset,
            })?;
    let mut event = StreamEvent::new(kind, sequence_number, request.output_index);
    event
        .fields
        .insert("item_id".to_owned(), EventField::from(item_id));
    Ok(event)
}

fn argument_events(
    request: &McpCallRequest,
    item_id: &str,
) -> Result<Vec<StreamEvent>, McpEventError> {
    let mut delta = call_event(request, item_id, McpEventKind::CallArgumentsDelta, 1)?;
    delta.fields.insert(
        "delta".to_owned(),
        EventField::from(request.arguments.as_str()),
    );
    let mut done = call_event(request, item_id, McpEventKind::CallArgumentsDone, 2)?;
    done.fields.insert(
        "arguments".to_owned(),
        EventField::from(request.arguments.as_str()),
    );
    Ok(vec![
        call_event(request, item_id, McpEventKind::CallInProgress, 0)?,
        delta,
        done,
    ])
}

fn output_item_done(
    request: &McpCallRequest,
    item_id: &str,
    output: &str,
    approval_request_id: String,
) -> StreamEvent {
    let mut done = StreamEvent::new(McpEventKind::OutputItemDone, 1, request.output_index);
    done.fields.insert(
        "item".to_owned(),
        EventField::from(EventItem::Call {
            id: item_id.to_owned(),
            approval_request_id,
            arguments: request.arguments.clone(),
            error: None,
            name: request.tool_name.clone(),
            output: output.to_owned(),
            server_label: "litellm".to_owned(),
        }),
    );
    done
}

pub fn create_mcp_call_events<Id>(
    request: &McpCallRequest,
    mut next_id: Id,
) -> Result<Vec<StreamEvent>, McpEventError>
where
    Id: FnMut() -> String,
{
    let item_id = match request.base_item_id.as_deref() {
        Some(supplied) if !supplied.is_empty() => supplied.to_owned(),
        _ => format!("mcp_{}", next_id()),
    };
    let mut events = argument_events(request, &item_id)?;
    match &request.result {
        None => events.push(call_event(request, &item_id, McpEventKind::CallFailed, 3)?),
        Some(output) => {
            events.push(call_event(
                request,
                &item_id,
                McpEventKind::CallCompleted,
                3,
            )?);
            let approval_request_id = format!("mcpr_{}", next_id());
            events.push(output_item_done(
                request,
                &item_id,
                output,
                approval_request_id,
            ));
        }
    }
    Ok(events)
}
