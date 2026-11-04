pub use super::super::responses_shapes::wire::WireDecodeLimits;
use super::super::responses_shapes::wire::decode_responses_shapes;
use betterlite_models::decisions::Json;
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponsesAPIStreamingResponse {
    Known {
        event: ResponsesAPIStreamEvents,
        fields: std::collections::BTreeMap<String, Json>,
    },
    Generic {
        event_type: String,
        fields: std::collections::BTreeMap<String, Json>,
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
    let mut value: Value = decode_responses_shapes(
        bytes,
        WireDecodeLimits {
            max_json_bytes: limits.max_json_bytes,
            max_event_bytes: limits.max_event_bytes,
            max_items: limits.max_items,
        },
    )?;
    let object = value
        .as_object()
        .ok_or(ResponsesEventDecodeError::MissingType)?;
    let event_type = object
        .get("type")
        .ok_or(ResponsesEventDecodeError::MissingType)?
        .as_str()
        .ok_or(ResponsesEventDecodeError::InvalidType)?
        .to_owned();
    let event =
        serde_json::from_value::<ResponsesAPIStreamEvents>(Value::String(event_type.clone()));
    if let Ok(event) = event {
        let Some(fields) = value.as_object_mut() else {
            return Err(ResponsesEventDecodeError::MissingType);
        };
        if matches!(
            event,
            ResponsesAPIStreamEvents::ReasoningSummaryTextDelta
                | ResponsesAPIStreamEvents::ReasoningSummaryTextDone
                | ResponsesAPIStreamEvents::ReasoningSummaryPartDone
        ) {
            fields
                .entry(String::from("summary_index"))
                .or_insert(Value::from(0));
        }
        if event == ResponsesAPIStreamEvents::OutputItemDone {
            fields
                .entry(String::from("sequence_number"))
                .or_insert(Value::from(1));
        }
        if event == ResponsesAPIStreamEvents::Error {
            if let Some(error) = fields.get_mut("error").and_then(Value::as_object_mut) {
                error.entry(String::from("param")).or_insert(Value::Null);
            }
        }
        for &field in required_fields(event) {
            if !fields.contains_key(field) {
                return Err(ResponsesEventDecodeError::MissingField(field.to_owned()));
            }
        }
        let fields = serde_json::from_value::<std::collections::BTreeMap<String, Json>>(value)?;
        if matches!(
            event,
            ResponsesAPIStreamEvents::CustomToolCallInputDelta
                | ResponsesAPIStreamEvents::CustomToolCallInputDone
                | ResponsesAPIStreamEvents::ShellCallInProgress
                | ResponsesAPIStreamEvents::ShellCallCompleted
                | ResponsesAPIStreamEvents::ShellCallOutput
        ) {
            Ok(ResponsesAPIStreamingResponse::Generic {
                event_type: event_type.clone(),
                fields,
            })
        } else {
            Ok(ResponsesAPIStreamingResponse::Known { event, fields })
        }
    } else {
        let fields = serde_json::from_value::<std::collections::BTreeMap<String, Json>>(value)?;
        Ok(ResponsesAPIStreamingResponse::Generic { event_type, fields })
    }
}

fn required_fields(event: ResponsesAPIStreamEvents) -> &'static [&'static str] {
    use ResponsesAPIStreamEvents as E;
    match event {
        E::ResponseCreated
        | E::ResponseInProgress
        | E::ResponseCompleted
        | E::ResponseFailed
        | E::ResponseIncomplete => &["response"],
        E::ResponsePartAdded => &["item_id", "output_index", "part"],
        E::ReasoningSummaryTextDelta => &["item_id", "output_index", "delta"],
        E::ReasoningSummaryTextDone => &["item_id", "output_index", "sequence_number", "text"],
        E::ReasoningSummaryPartDone => &["item_id", "output_index", "sequence_number", "part"],
        E::OutputItemAdded => &["output_index", "item"],
        E::OutputItemDone => &["output_index", "item"],
        E::ContentPartAdded | E::ContentPartDone => {
            &["item_id", "output_index", "content_index", "part"]
        }
        E::OutputTextDelta | E::RefusalDelta => {
            &["item_id", "output_index", "content_index", "delta"]
        }
        E::OutputTextAnnotationAdded => &[
            "item_id",
            "output_index",
            "content_index",
            "annotation_index",
            "annotation",
        ],
        E::OutputTextDone => &["item_id", "output_index", "content_index", "text"],
        E::RefusalDone => &["item_id", "output_index", "content_index", "refusal"],
        E::FunctionCallArgumentsDelta => &["item_id", "output_index", "delta"],
        E::FunctionCallArgumentsDone => &["item_id", "output_index", "arguments"],
        E::FileSearchCallInProgress
        | E::FileSearchCallSearching
        | E::FileSearchCallCompleted
        | E::WebSearchCallInProgress
        | E::WebSearchCallSearching
        | E::WebSearchCallCompleted => &["output_index", "item_id"],
        E::McpListToolsInProgress
        | E::McpListToolsCompleted
        | E::McpListToolsFailed
        | E::McpCallInProgress => &["sequence_number", "output_index", "item_id"],
        E::McpCallArgumentsDelta => &["output_index", "item_id", "delta", "sequence_number"],
        E::McpCallArgumentsDone => &["output_index", "item_id", "arguments", "sequence_number"],
        E::McpCallCompleted | E::McpCallFailed => &["sequence_number", "item_id", "output_index"],
        E::ImageGenerationPartialImage => &["partial_image_index", "b64_json"],
        E::Error => &["sequence_number", "error"],
        E::CustomToolCallInputDelta
        | E::CustomToolCallInputDone
        | E::ShellCallInProgress
        | E::ShellCallCompleted
        | E::ShellCallOutput => &["type"],
    }
}
