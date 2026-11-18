use betterlite_openai::api::responses_event_records::records::{
    KnownResponseEvent, ResponsesAPIStreamingResponse,
};

use super::kinds::EventKind;
use crate::api::responses_stream_events::JsonShape;
use crate::api::responses_stream_primitives::state::ResponsesStreamBaseState;

#[must_use]
pub fn delta_text(event: &ResponsesAPIStreamingResponse) -> Option<&str> {
    match event {
        ResponsesAPIStreamingResponse::Known(known) => match known.as_ref() {
            KnownResponseEvent::OutputTextDelta(record) => Some(&record.delta),
            KnownResponseEvent::FunctionCallArgumentsDelta(record) => Some(&record.delta),
            KnownResponseEvent::McpCallArgumentsDelta(record) => Some(&record.delta),
            KnownResponseEvent::ResponseCreated(_)
            | KnownResponseEvent::ResponseInProgress(_)
            | KnownResponseEvent::ResponseCompleted(_)
            | KnownResponseEvent::ResponseFailed(_)
            | KnownResponseEvent::ResponseIncomplete(_)
            | KnownResponseEvent::ResponsePartAdded(_)
            | KnownResponseEvent::ReasoningSummaryTextDelta(_)
            | KnownResponseEvent::ReasoningSummaryTextDone(_)
            | KnownResponseEvent::ReasoningSummaryPartDone(_)
            | KnownResponseEvent::OutputItemAdded(_)
            | KnownResponseEvent::OutputItemDone(_)
            | KnownResponseEvent::ContentPartAdded(_)
            | KnownResponseEvent::ContentPartDone(_)
            | KnownResponseEvent::OutputTextAnnotationAdded(_)
            | KnownResponseEvent::OutputTextDone(_)
            | KnownResponseEvent::RefusalDelta(_)
            | KnownResponseEvent::RefusalDone(_)
            | KnownResponseEvent::FunctionCallArgumentsDone(_)
            | KnownResponseEvent::FileSearchCallInProgress(_)
            | KnownResponseEvent::FileSearchCallSearching(_)
            | KnownResponseEvent::FileSearchCallCompleted(_)
            | KnownResponseEvent::WebSearchCallInProgress(_)
            | KnownResponseEvent::WebSearchCallSearching(_)
            | KnownResponseEvent::WebSearchCallCompleted(_)
            | KnownResponseEvent::McpListToolsInProgress(_)
            | KnownResponseEvent::McpListToolsCompleted(_)
            | KnownResponseEvent::McpListToolsFailed(_)
            | KnownResponseEvent::McpCallInProgress(_)
            | KnownResponseEvent::McpCallArgumentsDone(_)
            | KnownResponseEvent::McpCallCompleted(_)
            | KnownResponseEvent::McpCallFailed(_)
            | KnownResponseEvent::ImageGenerationPartialImage(_)
            | KnownResponseEvent::Error(_) => None,
        },
        ResponsesAPIStreamingResponse::Generic(record) => {
            record.extra.get("delta").and_then(JsonShape::optional_str)
        }
    }
}

pub fn accumulate(
    event: &ResponsesAPIStreamingResponse,
    kind: EventKind,
    state: &mut ResponsesStreamBaseState,
) {
    let Some(text) = delta_text(event) else {
        return;
    };
    match kind {
        EventKind::TextDelta => state.generated_content.push_str(text),
        EventKind::ArgsDelta => state.generated_tool_arguments.push_str(text),
        EventKind::Completed
        | EventKind::Incomplete
        | EventKind::Failed
        | EventKind::Item
        | EventKind::Annotation
        | EventKind::Part
        | EventKind::Other => {}
    }
}
