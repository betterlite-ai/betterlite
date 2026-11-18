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
            _ => None,
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
