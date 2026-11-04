use super::super::responses_shapes::wire::{WireDecodeLimits, decode_responses_shapes};
pub type GenericEvent = super::records::GenericEvent;
pub type ResponsesAPIStreamingResponse = super::records::ResponsesAPIStreamingResponse;
pub type ResponsesTerminalEvent = super::records::ResponsesTerminalEvent;
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ResponsesEventDecodeError {
    #[error("event JSON exceeds byte limit")]
    JsonTooLarge,
    #[error("invalid Responses event: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid Responses event: {0}")]
    Shape(#[from] super::super::responses_shapes::wire::ShapeDecodeError),
    #[error("event type is required")]
    MissingType,
    #[error("event type must be a string")]
    InvalidType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ResponsesAPIStreamEvents {
    #[serde(rename = "response.created")]
    ResponseCreated,
    #[serde(rename = "response.in_progress")]
    ResponseInProgress,
    #[serde(rename = "response.completed")]
    ResponseCompleted,
    #[serde(rename = "response.failed")]
    ResponseFailed,
    #[serde(rename = "response.incomplete")]
    ResponseIncomplete,
    #[serde(rename = "response.reasoning_summary_part.added")]
    ResponsePartAdded,
    #[serde(rename = "response.reasoning_summary_text.delta")]
    ReasoningSummaryTextDelta,
    #[serde(rename = "response.reasoning_summary_text.done")]
    ReasoningSummaryTextDone,
    #[serde(rename = "response.reasoning_summary_part.done")]
    ReasoningSummaryPartDone,
    #[serde(rename = "response.output_item.added")]
    OutputItemAdded,
    #[serde(rename = "response.output_item.done")]
    OutputItemDone,
    #[serde(rename = "response.content_part.added")]
    ContentPartAdded,
    #[serde(rename = "response.content_part.done")]
    ContentPartDone,
    #[serde(rename = "response.output_text.delta")]
    OutputTextDelta,
    #[serde(rename = "response.output_text.annotation.added")]
    OutputTextAnnotationAdded,
    #[serde(rename = "response.output_text.done")]
    OutputTextDone,
    #[serde(rename = "response.refusal.delta")]
    RefusalDelta,
    #[serde(rename = "response.refusal.done")]
    RefusalDone,
    #[serde(rename = "response.function_call_arguments.delta")]
    FunctionCallArgumentsDelta,
    #[serde(rename = "response.function_call_arguments.done")]
    FunctionCallArgumentsDone,
    #[serde(rename = "response.custom_tool_call_input.delta")]
    CustomToolCallInputDelta,
    #[serde(rename = "response.custom_tool_call_input.done")]
    CustomToolCallInputDone,
    #[serde(rename = "response.file_search_call.in_progress")]
    FileSearchCallInProgress,
    #[serde(rename = "response.file_search_call.searching")]
    FileSearchCallSearching,
    #[serde(rename = "response.file_search_call.completed")]
    FileSearchCallCompleted,
    #[serde(rename = "response.web_search_call.in_progress")]
    WebSearchCallInProgress,
    #[serde(rename = "response.web_search_call.searching")]
    WebSearchCallSearching,
    #[serde(rename = "response.web_search_call.completed")]
    WebSearchCallCompleted,
    #[serde(rename = "response.mcp_list_tools.in_progress")]
    McpListToolsInProgress,
    #[serde(rename = "response.mcp_list_tools.completed")]
    McpListToolsCompleted,
    #[serde(rename = "response.mcp_list_tools.failed")]
    McpListToolsFailed,
    #[serde(rename = "response.mcp_call.in_progress")]
    McpCallInProgress,
    #[serde(rename = "response.mcp_call_arguments.delta")]
    McpCallArgumentsDelta,
    #[serde(rename = "response.mcp_call_arguments.done")]
    McpCallArgumentsDone,
    #[serde(rename = "response.mcp_call.completed")]
    McpCallCompleted,
    #[serde(rename = "response.mcp_call.failed")]
    McpCallFailed,
    #[serde(rename = "image_generation.partial_image")]
    ImageGenerationPartialImage,
    #[serde(rename = "response.shell_call.in_progress")]
    ShellCallInProgress,
    #[serde(rename = "response.shell_call.completed")]
    ShellCallCompleted,
    #[serde(rename = "response.shell_call_output.done")]
    ShellCallOutput,
    #[serde(rename = "error")]
    Error,
}

pub fn decode_responses_event_record(
    bytes: &[u8],
    limits: WireDecodeLimits,
) -> Result<ResponsesAPIStreamingResponse, ResponsesEventDecodeError> {
    if bytes.len() > limits.max_json_bytes || bytes.len() > limits.max_event_bytes {
        return Err(ResponsesEventDecodeError::JsonTooLarge);
    }
    let value: Value = decode_responses_shapes(bytes, limits)?;
    decode_event(value)
}

fn decode_event(value: Value) -> Result<ResponsesAPIStreamingResponse, ResponsesEventDecodeError> {
    let event_type = event_type(&value)?;
    let parsed_event =
        serde_json::from_value::<ResponsesAPIStreamEvents>(Value::String(event_type));
    match parsed_event {
        Ok(known_event) if is_generic_event(known_event) => Ok(
            ResponsesAPIStreamingResponse::Generic(serde_json::from_value(value)?),
        ),
        Ok(_) => Ok(ResponsesAPIStreamingResponse::Known(Box::new(
            serde_json::from_value(value)?,
        ))),
        Err(_) => Ok(ResponsesAPIStreamingResponse::Generic(
            serde_json::from_value(value)?,
        )),
    }
}

fn event_type(value: &Value) -> Result<String, ResponsesEventDecodeError> {
    value
        .as_object()
        .ok_or(ResponsesEventDecodeError::MissingType)?
        .get("type")
        .ok_or(ResponsesEventDecodeError::MissingType)?
        .as_str()
        .map(str::to_owned)
        .ok_or(ResponsesEventDecodeError::InvalidType)
}

const fn is_generic_event(event: ResponsesAPIStreamEvents) -> bool {
    matches!(
        event,
        ResponsesAPIStreamEvents::CustomToolCallInputDelta
            | ResponsesAPIStreamEvents::CustomToolCallInputDone
            | ResponsesAPIStreamEvents::ShellCallInProgress
            | ResponsesAPIStreamEvents::ShellCallCompleted
            | ResponsesAPIStreamEvents::ShellCallOutput
    )
}
