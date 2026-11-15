use super::error::CompletionError;
use super::records::{CompletionEvent, FunctionCallArgumentsDone, ResponseState};
use super::tool_input::{ToolUseEvent, decode_realtime_tool_input};
use crate::agentcore_stream::IdSource;
use crate::realtime_session::records::Setting;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolUseOutput {
    pub events: Vec<CompletionEvent>,
    pub tool_call_id: String,
    pub tool_name: String,
}

impl ResponseState {
    // There is no output_item.added for the call and no check against the declared tools
    pub fn tool_use_events(
        &self,
        event: &ToolUseEvent,
        ids: &dyn IdSource,
    ) -> Result<ToolUseOutput, CompletionError> {
        let (Some(item_id), Some(response_id)) =
            (self.output_item_id.as_deref(), self.response_id.as_deref())
        else {
            return Ok(ToolUseOutput {
                events: Vec::new(),
                tool_call_id: String::new(),
                tool_name: String::new(),
            });
        };
        let arguments = match &event.tool_use.input {
            None => "{}".to_owned(),
            Some(Setting(None)) => "null".to_owned(),
            Some(Setting(Some(input))) => decode_realtime_tool_input(input.get().as_bytes())?
                .get()
                .to_owned(),
        };
        let tool_call_id = event.tool_use.tool_use_id.clone().unwrap_or_default();
        let tool_name = event.tool_use.tool_name.clone().unwrap_or_default();
        Ok(ToolUseOutput {
            events: vec![CompletionEvent::FunctionCallArgumentsDone(
                FunctionCallArgumentsDone {
                    event_id: format!("event_{}", ids.next_id()),
                    response_id: response_id.to_owned(),
                    item_id: item_id.to_owned(),
                    output_index: 0,
                    call_id: tool_call_id.clone(),
                    name: tool_name.clone(),
                    arguments,
                },
            )],
            tool_call_id,
            tool_name,
        })
    }
}
