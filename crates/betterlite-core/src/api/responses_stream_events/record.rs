use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::event_types::{
    ContentPartDonePart, Extra, RequiredNullable,
};
use betterlite_openai::api::responses_event_records::records::{
    KnownResponseEvent, ResponsesAPIStreamingResponse,
};
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use indexmap::IndexMap;

use crate::api::response_id_traversal::record::{DictRecord, Record};

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
    match event {
        ResponsesAPIStreamingResponse::Known(known) => {
            if let KnownResponseEvent::ResponseCreated(record)
            | KnownResponseEvent::ResponseInProgress(record)
            | KnownResponseEvent::ResponseCompleted(record)
            | KnownResponseEvent::ResponseFailed(record)
            | KnownResponseEvent::ResponseIncomplete(record) = known.as_mut()
            {
                return Some(ResponseTarget::Typed(&mut record.response));
            }
            known_extra(known.as_mut())
                .and_then(|extra| extra.get_mut("response"))
                .map(ResponseTarget::Value)
        }
        ResponsesAPIStreamingResponse::Generic(record) => {
            record.extra.get_mut("response").map(ResponseTarget::Value)
        }
    }
}

// the event's own kind names the one slot whose container ids are encoded
#[must_use]
pub fn slot_target(event: &mut ResponsesAPIStreamingResponse) -> Option<Target<'_>> {
    match event {
        ResponsesAPIStreamingResponse::Known(known) => known_target(known.as_mut()),
        ResponsesAPIStreamingResponse::Generic(record) => {
            generic_target(&mut record.extra, &record.r#type)
        }
    }
}

fn known_target(event: &mut KnownResponseEvent) -> Option<Target<'_>> {
    if let KnownResponseEvent::OutputItemAdded(record) = event {
        return match &mut record.item {
            RequiredNullable::Value(extra) => Some(Target::Extra(extra)),
            RequiredNullable::Null => None,
        };
    }
    if let KnownResponseEvent::OutputItemDone(record) = event {
        return Some(Target::Extra(&mut record.item));
    }
    if let KnownResponseEvent::OutputTextAnnotationAdded(record) = event {
        return Some(Target::Extra(&mut record.annotation));
    }
    if let KnownResponseEvent::ContentPartDone(record) = event {
        return match &mut record.part {
            ContentPartDonePart::OutputText { annotations, .. } => {
                Some(Target::Extras(annotations))
            }
            ContentPartDonePart::Refusal { .. } | ContentPartDonePart::ReasoningText { .. } => None,
        };
    }
    None
}

fn generic_target<'event>(extra: &'event mut Extra, event_type: &str) -> Option<Target<'event>> {
    match event_type {
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

// serde's untagged Record decode reads a float as its private number map and the Dict arm wins, so a JSON value converts by hand
#[must_use]
pub fn record_of(value: &Json) -> Record {
    if let Json::Object(fields) = value {
        return record_of_fields(fields.iter());
    }
    if let Json::List(items) = value {
        return Record::List(items.iter().map(record_of).collect());
    }
    Record::Scalar(value.clone())
}

#[must_use]
pub fn record_of_fields<'field, Fields>(fields: Fields) -> Record
where
    Fields: Iterator<Item = (&'field String, &'field Json)>,
{
    Record::Dict(DictRecord {
        values: fields
            .map(|(name, child)| (name.clone(), record_of(child)))
            .collect(),
    })
}

#[must_use]
pub fn json_of(record: &Record) -> Json {
    match record {
        Record::Dict(dict) => object_of(&dict.values),
        Record::Object { object } => object_of(&object.fields),
        Record::List(items) => Json::List(items.iter().map(json_of).collect()),
        Record::Scalar(value) => value.clone(),
    }
}

fn object_of(fields: &IndexMap<String, Record>) -> Json {
    Json::Object(
        fields
            .iter()
            .map(|(name, child)| (name.clone(), json_of(child)))
            .collect(),
    )
}
