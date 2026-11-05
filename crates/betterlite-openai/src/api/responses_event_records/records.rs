use super::event_types::{
    ContentPartAddedEvent, ContentPartDoneEvent, ErrorEvent, Extra, FunctionArgsDelta,
    FunctionArgsDone, ImagePartial, IndexedItemEvent, McpCallArgumentsDeltaEvent,
    McpCallArgumentsDoneEvent, McpCallTerminalEvent, McpListToolsEvent, OutputItemAddedEvent,
    OutputItemDoneEvent, OutputTextAnnotationAddedEvent, OutputTextDeltaEvent, OutputTextDoneEvent,
    ReasoningSummaryPartDoneEvent, ReasoningSummaryTextDeltaEvent, ReasoningSummaryTextDoneEvent,
    RefusalDeltaEvent, RefusalDoneEvent, ResponseEvent, ResponsePartAddedEvent,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum KnownResponseEvent {
    #[serde(rename = "response.created")]
    ResponseCreated(ResponseEvent),
    #[serde(rename = "response.in_progress")]
    ResponseInProgress(ResponseEvent),
    #[serde(rename = "response.completed")]
    ResponseCompleted(ResponseEvent),
    #[serde(rename = "response.failed")]
    ResponseFailed(ResponseEvent),
    #[serde(rename = "response.incomplete")]
    ResponseIncomplete(ResponseEvent),
    #[serde(rename = "response.reasoning_summary_part.added")]
    ResponsePartAdded(ResponsePartAddedEvent),
    #[serde(rename = "response.reasoning_summary_text.delta")]
    ReasoningSummaryTextDelta(ReasoningSummaryTextDeltaEvent),
    #[serde(rename = "response.reasoning_summary_text.done")]
    ReasoningSummaryTextDone(ReasoningSummaryTextDoneEvent),
    #[serde(rename = "response.reasoning_summary_part.done")]
    ReasoningSummaryPartDone(ReasoningSummaryPartDoneEvent),
    #[serde(rename = "response.output_item.added")]
    OutputItemAdded(OutputItemAddedEvent),
    #[serde(rename = "response.output_item.done")]
    OutputItemDone(OutputItemDoneEvent),
    #[serde(rename = "response.content_part.added")]
    ContentPartAdded(ContentPartAddedEvent),
    #[serde(rename = "response.content_part.done")]
    ContentPartDone(ContentPartDoneEvent),
    #[serde(rename = "response.output_text.delta")]
    OutputTextDelta(OutputTextDeltaEvent),
    #[serde(rename = "response.output_text.annotation.added")]
    OutputTextAnnotationAdded(OutputTextAnnotationAddedEvent),
    #[serde(rename = "response.output_text.done")]
    OutputTextDone(OutputTextDoneEvent),
    #[serde(rename = "response.refusal.delta")]
    RefusalDelta(RefusalDeltaEvent),
    #[serde(rename = "response.refusal.done")]
    RefusalDone(RefusalDoneEvent),
    #[serde(rename = "response.function_call_arguments.delta")]
    FunctionCallArgumentsDelta(FunctionArgsDelta),
    #[serde(rename = "response.function_call_arguments.done")]
    FunctionCallArgumentsDone(FunctionArgsDone),
    #[serde(rename = "response.file_search_call.in_progress")]
    FileSearchCallInProgress(IndexedItemEvent),
    #[serde(rename = "response.file_search_call.searching")]
    FileSearchCallSearching(IndexedItemEvent),
    #[serde(rename = "response.file_search_call.completed")]
    FileSearchCallCompleted(IndexedItemEvent),
    #[serde(rename = "response.web_search_call.in_progress")]
    WebSearchCallInProgress(IndexedItemEvent),
    #[serde(rename = "response.web_search_call.searching")]
    WebSearchCallSearching(IndexedItemEvent),
    #[serde(rename = "response.web_search_call.completed")]
    WebSearchCallCompleted(IndexedItemEvent),
    #[serde(rename = "response.mcp_list_tools.in_progress")]
    McpListToolsInProgress(McpListToolsEvent),
    #[serde(rename = "response.mcp_list_tools.completed")]
    McpListToolsCompleted(McpListToolsEvent),
    #[serde(rename = "response.mcp_list_tools.failed")]
    McpListToolsFailed(McpListToolsEvent),
    #[serde(rename = "response.mcp_call.in_progress")]
    McpCallInProgress(McpListToolsEvent),
    #[serde(rename = "response.mcp_call_arguments.delta")]
    McpCallArgumentsDelta(McpCallArgumentsDeltaEvent),
    #[serde(rename = "response.mcp_call_arguments.done")]
    McpCallArgumentsDone(McpCallArgumentsDoneEvent),
    #[serde(rename = "response.mcp_call.completed")]
    McpCallCompleted(McpCallTerminalEvent),
    #[serde(rename = "response.mcp_call.failed")]
    McpCallFailed(McpCallTerminalEvent),
    #[serde(rename = "image_generation.partial_image")]
    ImageGenerationPartialImage(ImagePartial),
    #[serde(rename = "error")]
    Error(ErrorEvent),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResponsesAPIStreamingResponse {
    Known(Box<KnownResponseEvent>),
    Generic(GenericEvent),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenericEvent {
    pub r#type: String,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ResponsesTerminalEvent {
    #[serde(rename = "response.completed")]
    Completed(ResponseEvent),
    #[serde(rename = "response.incomplete")]
    Incomplete(ResponseEvent),
    #[serde(rename = "response.failed")]
    Failed(ResponseEvent),
}
