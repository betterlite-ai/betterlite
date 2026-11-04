use super::super::responses_shapes::wire::{WireDecodeLimits, decode_responses_shapes};
use betterlite_models::decisions::Json;
use serde_json::Value;
use std::collections::BTreeMap;
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
    #[error("required event field is missing: {0}")]
    MissingField(String),
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

use ResponsesAPIStreamEvents as Event;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponsesAPIStreamingResponse {
    Known {
        event: ResponsesAPIStreamEvents,
        fields: BTreeMap<String, Json>,
    },
    Generic {
        event_type: String,
        fields: BTreeMap<String, Json>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponsesTerminalEvent {
    Completed(ResponsesAPIStreamingResponse),
    Incomplete(ResponsesAPIStreamingResponse),
    Failed(ResponsesAPIStreamingResponse),
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
        serde_json::from_value::<ResponsesAPIStreamEvents>(Value::String(event_type.clone()));
    match parsed_event {
        Ok(known_event) => decode_known_event(known_event, event_type, value),
        Err(_) => Ok(ResponsesAPIStreamingResponse::Generic {
            event_type,
            fields: serde_json::from_value::<BTreeMap<String, Json>>(value)?,
        }),
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

fn decode_known_event(
    event: ResponsesAPIStreamEvents,
    event_type: String,
    mut value: Value,
) -> Result<ResponsesAPIStreamingResponse, ResponsesEventDecodeError> {
    let fields = value
        .as_object_mut()
        .ok_or(ResponsesEventDecodeError::MissingType)?;
    add_event_defaults(event, fields);
    validate_required_fields(event, fields)?;
    let decoded_fields = serde_json::from_value::<BTreeMap<String, Json>>(value)?;
    if is_generic_event(event) {
        Ok(ResponsesAPIStreamingResponse::Generic {
            event_type,
            fields: decoded_fields,
        })
    } else {
        Ok(ResponsesAPIStreamingResponse::Known {
            event,
            fields: decoded_fields,
        })
    }
}

fn add_event_defaults(
    event: ResponsesAPIStreamEvents,
    fields: &mut serde_json::Map<String, Value>,
) {
    if matches!(
        event,
        ResponsesAPIStreamEvents::ReasoningSummaryTextDelta
            | ResponsesAPIStreamEvents::ReasoningSummaryTextDone
            | ResponsesAPIStreamEvents::ReasoningSummaryPartDone
    ) {
        fields
            .entry(String::from("summary_index"))
            .or_insert(Value::from(0_i64));
    }
    if event == ResponsesAPIStreamEvents::OutputItemDone {
        fields
            .entry(String::from("sequence_number"))
            .or_insert(Value::from(1_i64));
    }
    if event == ResponsesAPIStreamEvents::Error
        && let Some(error) = fields.get_mut("error").and_then(Value::as_object_mut)
    {
        error.entry(String::from("param")).or_insert(Value::Null);
    }
}

fn validate_required_fields(
    event: ResponsesAPIStreamEvents,
    fields: &serde_json::Map<String, Value>,
) -> Result<(), ResponsesEventDecodeError> {
    for &field in event_fields(event_field_set(event)) {
        if !fields.contains_key(field) {
            return Err(ResponsesEventDecodeError::MissingField(field.to_owned()));
        }
    }
    Ok(())
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

#[derive(Clone, Copy)]
enum EventFieldSet {
    Response,
    Part,
    Delta,
    SequenceText,
    SequencePart,
    Item,
    ContentPart,
    IndexedDelta,
    Annotation,
    Text,
    Refusal,
    Arguments,
    Search,
    Mcp,
    McpDelta,
    McpArguments,
    McpTerminal,
    Image,
    Error,
    Type,
}

const fn event_field_set(event: ResponsesAPIStreamEvents) -> EventFieldSet {
    match event {
        Event::ResponseCreated
        | Event::ResponseInProgress
        | Event::ResponseCompleted
        | Event::ResponseFailed
        | Event::ResponseIncomplete => EventFieldSet::Response,
        Event::ResponsePartAdded => EventFieldSet::Part,
        Event::ReasoningSummaryTextDelta | Event::FunctionCallArgumentsDelta => {
            EventFieldSet::Delta
        }
        Event::ReasoningSummaryTextDone => EventFieldSet::SequenceText,
        Event::ReasoningSummaryPartDone => EventFieldSet::SequencePart,
        Event::OutputItemAdded | Event::OutputItemDone => EventFieldSet::Item,
        Event::ContentPartAdded | Event::ContentPartDone => EventFieldSet::ContentPart,
        Event::OutputTextDelta | Event::RefusalDelta => EventFieldSet::IndexedDelta,
        Event::OutputTextAnnotationAdded => EventFieldSet::Annotation,
        Event::OutputTextDone => EventFieldSet::Text,
        Event::RefusalDone => EventFieldSet::Refusal,
        Event::FunctionCallArgumentsDone => EventFieldSet::Arguments,
        Event::FileSearchCallInProgress
        | Event::FileSearchCallSearching
        | Event::FileSearchCallCompleted
        | Event::WebSearchCallInProgress
        | Event::WebSearchCallSearching
        | Event::WebSearchCallCompleted => EventFieldSet::Search,
        Event::McpListToolsInProgress
        | Event::McpListToolsCompleted
        | Event::McpListToolsFailed
        | Event::McpCallInProgress => EventFieldSet::Mcp,
        Event::McpCallArgumentsDelta => EventFieldSet::McpDelta,
        Event::McpCallArgumentsDone => EventFieldSet::McpArguments,
        Event::McpCallCompleted | Event::McpCallFailed => EventFieldSet::McpTerminal,
        Event::ImageGenerationPartialImage => EventFieldSet::Image,
        Event::Error => EventFieldSet::Error,
        Event::CustomToolCallInputDelta
        | Event::CustomToolCallInputDone
        | Event::ShellCallInProgress
        | Event::ShellCallCompleted
        | Event::ShellCallOutput => EventFieldSet::Type,
    }
}

const fn event_fields(fields: EventFieldSet) -> &'static [&'static str] {
    match fields {
        EventFieldSet::Response => &["response"],
        EventFieldSet::Part => &["item_id", "output_index", "part"],
        EventFieldSet::Delta => &["item_id", "output_index", "delta"],
        EventFieldSet::SequenceText => &["item_id", "output_index", "sequence_number", "text"],
        EventFieldSet::SequencePart => &["item_id", "output_index", "sequence_number", "part"],
        EventFieldSet::Item => &["output_index", "item"],
        EventFieldSet::ContentPart => &["item_id", "output_index", "content_index", "part"],
        EventFieldSet::IndexedDelta => &["item_id", "output_index", "content_index", "delta"],
        EventFieldSet::Annotation => &[
            "item_id",
            "output_index",
            "content_index",
            "annotation_index",
            "annotation",
        ],
        EventFieldSet::Text => &["item_id", "output_index", "content_index", "text"],
        EventFieldSet::Refusal => &["item_id", "output_index", "content_index", "refusal"],
        EventFieldSet::Arguments => &["item_id", "output_index", "arguments"],
        EventFieldSet::Search => SEARCH_FIELDS,
        EventFieldSet::Mcp => MCP_FIELDS,
        EventFieldSet::McpDelta => &["output_index", "item_id", "delta", "sequence_number"],
        EventFieldSet::McpArguments => &["output_index", "item_id", "arguments", "sequence_number"],
        EventFieldSet::McpTerminal => &["sequence_number", "item_id", "output_index"],
        EventFieldSet::Image => &["partial_image_index", "b64_json"],
        EventFieldSet::Error => &["sequence_number", "error"],
        EventFieldSet::Type => &["type"],
    }
}

const SEARCH_FIELDS: &[&str] = &["output_index", "item_id"];
const MCP_FIELDS: &[&str] = &["sequence_number", "output_index", "item_id"];
