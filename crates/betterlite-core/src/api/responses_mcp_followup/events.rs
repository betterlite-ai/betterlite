use betterlite_models::decisions::Json;

use crate::api::responses_mcp_events::call::{McpCallRequest, create_mcp_call_events};
use crate::api::responses_mcp_events::error::McpEventError;
use crate::api::responses_mcp_events::event::StreamEvent;
use crate::api::responses_mcp_results::McpToolResult;
use crate::api::responses_mcp_tool_selection::{
    McpToolCall, McpToolCallDetails, extract_tool_call_details,
};

use super::SHORT_ID_LENGTH;
use super::error::ToolExecutionEventError;
use super::records::IdSource;

const UNKNOWN_TOOL_NAME: &str = "unknown";
const UNKNOWN_TOOL_CALL_ID: &str = "unknown";
const EMPTY_ARGUMENTS: &str = "{}";
const ITEM_ID_PREFIX: &str = "mcp_";
const OUTPUT_INDEX: i64 = 0;

// B952's create_mcp_call_events behind a seam, so a test can read the request and the ids the flow hands it
pub trait CallEventsBuilder {
    fn create(
        &self,
        request: &McpCallRequest,
        next_id: &mut dyn FnMut() -> String,
    ) -> Result<Vec<StreamEvent>, McpEventError>;
}

pub struct LiteLlmCallEvents;

impl CallEventsBuilder for LiteLlmCallEvents {
    fn create(
        &self,
        request: &McpCallRequest,
        next_id: &mut dyn FnMut() -> String,
    ) -> Result<Vec<StreamEvent>, McpEventError> {
        create_mcp_call_events(request, next_id)
    }
}

// tool_result.get("tool_call_id", "unknown"): a missing key was defaulted, an explicit null stayed null
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ToolCallId {
    #[default]
    Absent,
    Null,
    Value(String),
}

impl ToolCallId {
    fn matches(&self, call_id: Option<&str>) -> bool {
        match self {
            Self::Absent => call_id == Some(UNKNOWN_TOOL_CALL_ID),
            Self::Null => call_id.is_none(),
            Self::Value(found) => call_id == Some(found.as_str()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventToolResult {
    pub tool_call_id: ToolCallId,
    pub result: String,
}

impl From<&McpToolResult> for EventToolResult {
    fn from(result: &McpToolResult) -> Self {
        Self {
            tool_call_id: match &result.tool_call_id {
                Some(id) => ToolCallId::Value(id.clone()),
                None => ToolCallId::Null,
            },
            result: result.result.clone(),
        }
    }
}

// one five-event group per tool result, starting after every event built so far (L1224-L1272)
pub fn create_tool_execution_events<Builder>(
    builder: &Builder,
    tool_calls: &[McpToolCall],
    tool_results: &[EventToolResult],
    ids: &IdSource,
) -> Result<Vec<StreamEvent>, ToolExecutionEventError>
where
    Builder: CallEventsBuilder + ?Sized,
{
    let mut events: Vec<StreamEvent> = Vec::new();
    for result in tool_results {
        let details = matching_details(tool_calls, &result.tool_call_id)?;
        let (tool_name, arguments) = event_arguments(details.as_ref())?;
        let completed = i64::try_from(events.len()).unwrap_or(i64::MAX);
        let suffix: String = ids().chars().take(SHORT_ID_LENGTH).collect();
        let request = McpCallRequest {
            tool_name,
            arguments,
            result: Some(result.result.clone()),
            base_item_id: Some(format!("{ITEM_ID_PREFIX}{suffix}")),
            sequence_start: completed.saturating_add(1),
            output_index: OUTPUT_INDEX,
        };
        let mut next_id = || ids();
        let built = builder.create(&request, &mut next_id)?;
        events.extend(built);
    }
    Ok(events)
}

fn matching_details(
    tool_calls: &[McpToolCall],
    tool_call_id: &ToolCallId,
) -> Result<Option<McpToolCallDetails>, ToolExecutionEventError> {
    for tool_call in tool_calls {
        let details = extract_tool_call_details(tool_call)?;
        if tool_call_id.matches(details.call_id.as_deref()) {
            return Ok(Some(details));
        }
    }
    Ok(None)
}

// name or "unknown" and args or "{}", with a falsy name or empty/absent arguments taking the default
fn event_arguments(
    details: Option<&McpToolCallDetails>,
) -> Result<(String, String), ToolExecutionEventError> {
    let Some(details) = details else {
        return Ok((UNKNOWN_TOOL_NAME.to_owned(), EMPTY_ARGUMENTS.to_owned()));
    };
    let tool_name = details
        .name
        .as_deref()
        .filter(|text| !text.is_empty())
        .map_or_else(|| UNKNOWN_TOOL_NAME.to_owned(), str::to_owned);
    Ok((tool_name, arguments_text(details.arguments.as_ref())?))
}

fn arguments_text(arguments: Option<&Json>) -> Result<String, ToolExecutionEventError> {
    match arguments {
        None | Some(Json::Null) => Ok(EMPTY_ARGUMENTS.to_owned()),
        Some(Json::Text(text)) if text.is_empty() => Ok(EMPTY_ARGUMENTS.to_owned()),
        Some(Json::Text(text)) => Ok(text.clone()),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(ToolExecutionEventError::ArgumentsNotString)
        }
    }
}
