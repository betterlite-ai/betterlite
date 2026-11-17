use betterlite_prompts::arguments::json::Object;
use serde::Serialize;
use serde_json::Number;

use crate::gemini_live_accounting::tool_calls::FunctionCallArgumentsDone;
use crate::gemini_live_setup::session::MaxResponseTokens;

// One tool turn is a flat sequence, so each variant serializes as the event object the client reads
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ToolTurnEvent {
    Created(ResponseCreatedEvent),
    OutputItemAdded(OutputItemEvent),
    ConversationItemAdded(ConversationItemAddedEvent),
    ArgumentsDelta(ArgumentsDeltaEvent),
    ArgumentsDone(FunctionCallArgumentsDone),
    OutputItemDone(OutputItemEvent),
    Done(ToolTurnDoneEvent),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ResponseCreatedEvent {
    #[serde(rename = "type")]
    pub event_type: &'static str,
    pub event_id: String,
    pub response: ResponseCreatedObject,
}

// Both limits are present here even when the setup left them unset, which the done object does not do
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ResponseCreatedObject {
    pub object: &'static str,
    pub id: String,
    pub status: &'static str,
    pub status_details: Option<Object>,
    pub output: Vec<Object>,
    pub conversation_id: String,
    pub modalities: Vec<String>,
    pub temperature: Option<Number>,
    pub max_output_tokens: Option<MaxResponseTokens>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct FunctionCallItem {
    pub id: String,
    pub object: &'static str,
    #[serde(rename = "type")]
    pub item_type: &'static str,
    pub status: &'static str,
    pub call_id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct OutputItemEvent {
    #[serde(rename = "type")]
    pub event_type: &'static str,
    pub event_id: String,
    pub response_id: String,
    pub output_index: usize,
    pub item: FunctionCallItem,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ConversationItemAddedEvent {
    #[serde(rename = "type")]
    pub event_type: &'static str,
    pub event_id: String,
    pub previous_item_id: Option<String>,
    pub item: FunctionCallItem,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ArgumentsDeltaEvent {
    #[serde(rename = "type")]
    pub event_type: &'static str,
    pub event_id: String,
    pub response_id: String,
    pub item_id: String,
    pub output_index: usize,
    pub call_id: String,
    pub delta: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ToolTurnDoneEvent {
    #[serde(rename = "type")]
    pub event_type: &'static str,
    pub event_id: String,
    pub response: ToolTurnDoneResponse,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ToolTurnDoneResponse {
    pub object: &'static str,
    pub id: String,
    pub status: &'static str,
    pub status_details: Option<Object>,
    pub output: Vec<FunctionCallItem>,
    pub conversation_id: String,
    pub modalities: Vec<String>,
    pub usage: Object,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<Number>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<MaxResponseTokens>,
}
