use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::event_types::{
    ContentPartDonePart, Extra, RequiredNullable,
};
use betterlite_openai::api::responses_event_records::records::{
    GenericEvent, KnownResponseEvent, ResponsesAPIStreamingResponse,
};
use betterlite_openai::api::responses_objects::ResponsesApiResponse;

use crate::api::responses_stream_hooks::events::nested_response_mut;

pub enum Target<'event> {
    Extra(&'event mut Extra),
    Extras(&'event mut Vec<Extra>),
    Value(&'event mut Json),
    Values(&'event mut Json),
}

pub enum ResponseTarget<'event> {
    Typed(&'event mut ResponsesApiResponse),
    Value(&'event mut Json),
}

// a JSON null stays a value here; the rewrite itself ignores anything that is not an object
#[must_use]
pub fn response_target(event: &mut ResponsesAPIStreamingResponse) -> Option<ResponseTarget<'_>> {
    if let Some(response) = nested_response_mut(event) {
        return Some(ResponseTarget::Typed(response));
    }
    let extra = match event {
        ResponsesAPIStreamingResponse::Known(known) => known_extra(known.as_mut()),
        ResponsesAPIStreamingResponse::Generic(record) => Some(&mut record.extra),
    }?;
    extra.get_mut("response").map(ResponseTarget::Value)
}

// the event's own kind names the one slot whose container ids are encoded
#[must_use]
pub fn slot_target(event: &mut ResponsesAPIStreamingResponse) -> Option<Target<'_>> {
    match event {
        ResponsesAPIStreamingResponse::Known(known) => known_target(known.as_mut()),
        ResponsesAPIStreamingResponse::Generic(record) => generic_target(record),
    }
}

fn known_target(event: &mut KnownResponseEvent) -> Option<Target<'_>> {
    match event {
        KnownResponseEvent::OutputItemAdded(record) => match &mut record.item {
            RequiredNullable::Value(extra) => Some(Target::Extra(extra)),
            RequiredNullable::Null => None,
        },
        KnownResponseEvent::OutputItemDone(record) => Some(Target::Extra(&mut record.item)),
        KnownResponseEvent::OutputTextAnnotationAdded(record) => {
            Some(Target::Extra(&mut record.annotation))
        }
        KnownResponseEvent::ContentPartDone(record) => match &mut record.part {
            ContentPartDonePart::OutputText { annotations, .. } => {
                Some(Target::Extras(annotations))
            }
            ContentPartDonePart::Refusal { .. } | ContentPartDonePart::ReasoningText { .. } => None,
        },
        _ => None,
    }
}

fn generic_target(record: &mut GenericEvent) -> Option<Target<'_>> {
    let GenericEvent { r#type, extra } = record;
    match r#type.as_str() {
        "response.output_item.added" | "response.output_item.done" => {
            Some(Target::Value(extra.get_mut("item")?))
        }
        "response.output_text.annotation.added" => {
            Some(Target::Value(extra.get_mut("annotation")?))
        }
        "response.content_part.done" => {
            let Json::Object(fields) = extra.get_mut("part")? else {
                return None;
            };
            fields.get_mut("annotations").map(Target::Values)
        }
        _ => None,
    }
}

// a pydantic event model answers getattr("response") from its allowed extras, so any variant may carry one
fn known_extra(event: &mut KnownResponseEvent) -> Option<&mut Extra> {
    match event {
        KnownResponseEvent::ResponseCreated(_)
        | KnownResponseEvent::ResponseInProgress(_)
        | KnownResponseEvent::ResponseCompleted(_)
        | KnownResponseEvent::ResponseFailed(_)
        | KnownResponseEvent::ResponseIncomplete(_) => None,
        KnownResponseEvent::ResponsePartAdded(record) => Some(&mut record.extra),
        KnownResponseEvent::ReasoningSummaryTextDelta(record) => Some(&mut record.extra),
        KnownResponseEvent::ReasoningSummaryTextDone(record) => Some(&mut record.extra),
        KnownResponseEvent::ReasoningSummaryPartDone(record) => Some(&mut record.extra),
        KnownResponseEvent::OutputItemAdded(record) => Some(&mut record.extra),
        KnownResponseEvent::OutputItemDone(record) => Some(&mut record.extra),
        KnownResponseEvent::ContentPartAdded(record) => Some(&mut record.extra),
        KnownResponseEvent::ContentPartDone(record) => Some(&mut record.extra),
        KnownResponseEvent::OutputTextDelta(record) => Some(&mut record.extra),
        KnownResponseEvent::OutputTextAnnotationAdded(record) => Some(&mut record.extra),
        KnownResponseEvent::OutputTextDone(record) => Some(&mut record.extra),
        KnownResponseEvent::RefusalDelta(record) => Some(&mut record.extra),
        KnownResponseEvent::RefusalDone(record) => Some(&mut record.extra),
        KnownResponseEvent::FunctionCallArgumentsDelta(record) => Some(&mut record.extra),
        KnownResponseEvent::FunctionCallArgumentsDone(record) => Some(&mut record.extra),
        KnownResponseEvent::FileSearchCallInProgress(record)
        | KnownResponseEvent::FileSearchCallSearching(record)
        | KnownResponseEvent::FileSearchCallCompleted(record)
        | KnownResponseEvent::WebSearchCallInProgress(record)
        | KnownResponseEvent::WebSearchCallSearching(record)
        | KnownResponseEvent::WebSearchCallCompleted(record) => Some(&mut record.extra),
        KnownResponseEvent::McpListToolsInProgress(record)
        | KnownResponseEvent::McpListToolsCompleted(record)
        | KnownResponseEvent::McpListToolsFailed(record)
        | KnownResponseEvent::McpCallInProgress(record) => Some(&mut record.extra),
        KnownResponseEvent::McpCallArgumentsDelta(record) => Some(&mut record.extra),
        KnownResponseEvent::McpCallArgumentsDone(record) => Some(&mut record.extra),
        KnownResponseEvent::McpCallCompleted(record)
        | KnownResponseEvent::McpCallFailed(record) => Some(&mut record.extra),
        KnownResponseEvent::ImageGenerationPartialImage(record) => Some(&mut record.extra),
        KnownResponseEvent::Error(record) => Some(&mut record.extra),
    }
}
