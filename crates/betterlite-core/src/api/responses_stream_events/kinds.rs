use betterlite_openai::api::responses_event_records::records::{
    KnownResponseEvent, ResponsesAPIStreamingResponse,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    Completed,
    Incomplete,
    Failed,
    Item,
    Annotation,
    Part,
    TextDelta,
    ArgsDelta,
    Other,
}

#[must_use]
pub fn event_kind(event: &ResponsesAPIStreamingResponse) -> EventKind {
    match event {
        ResponsesAPIStreamingResponse::Known(known) => match known.as_ref() {
            KnownResponseEvent::ResponseCompleted(_) => EventKind::Completed,
            KnownResponseEvent::ResponseIncomplete(_) => EventKind::Incomplete,
            KnownResponseEvent::ResponseFailed(_) => EventKind::Failed,
            KnownResponseEvent::OutputItemAdded(_) | KnownResponseEvent::OutputItemDone(_) => {
                EventKind::Item
            }
            KnownResponseEvent::OutputTextAnnotationAdded(_) => EventKind::Annotation,
            KnownResponseEvent::ContentPartDone(_) => EventKind::Part,
            KnownResponseEvent::OutputTextDelta(_) => EventKind::TextDelta,
            KnownResponseEvent::FunctionCallArgumentsDelta(_)
            | KnownResponseEvent::McpCallArgumentsDelta(_) => EventKind::ArgsDelta,
            _ => EventKind::Other,
        },
        ResponsesAPIStreamingResponse::Generic(generic) => match generic.r#type.as_str() {
            "response.completed" => EventKind::Completed,
            "response.incomplete" => EventKind::Incomplete,
            "response.failed" => EventKind::Failed,
            "response.output_item.added" | "response.output_item.done" => EventKind::Item,
            "response.output_text.annotation.added" => EventKind::Annotation,
            "response.content_part.done" => EventKind::Part,
            "response.output_text.delta" => EventKind::TextDelta,
            "response.function_call_arguments.delta"
            | "response.custom_tool_call_input.delta"
            | "response.mcp_call_arguments.delta" => EventKind::ArgsDelta,
            _ => EventKind::Other,
        },
    }
}

#[must_use]
pub fn is_terminal(kind: EventKind) -> bool {
    match kind {
        EventKind::Completed | EventKind::Incomplete | EventKind::Failed => true,
        EventKind::Item
        | EventKind::Annotation
        | EventKind::Part
        | EventKind::TextDelta
        | EventKind::ArgsDelta
        | EventKind::Other => false,
    }
}
