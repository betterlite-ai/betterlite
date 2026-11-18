use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::event_types::{
    ContentPartDonePart, Extra, RequiredNullable,
};
use betterlite_openai::api::responses_event_records::records::{
    KnownResponseEvent, ResponsesAPIStreamingResponse,
};
use betterlite_openai::api::responses_objects::ResponsesApiResponse;

use crate::api::response_id_traversal::record::{DictRecord, Record};

pub enum Target<'event> {
    Extra(&'event mut Extra),
    Value(&'event mut Json),
}

pub enum PartTarget<'event> {
    Extra(&'event mut Vec<Extra>),
    Value(&'event mut Json),
}

pub enum ResponseTarget<'event> {
    Typed(&'event mut ResponsesApiResponse),
    Value(&'event mut Json),
}

#[must_use]
pub fn response_target(event: &mut ResponsesAPIStreamingResponse) -> Option<ResponseTarget<'_>> {
    match event {
        ResponsesAPIStreamingResponse::Known(known) => match known.as_mut() {
            KnownResponseEvent::ResponseCreated(record)
            | KnownResponseEvent::ResponseInProgress(record)
            | KnownResponseEvent::ResponseCompleted(record)
            | KnownResponseEvent::ResponseFailed(record)
            | KnownResponseEvent::ResponseIncomplete(record) => {
                Some(ResponseTarget::Typed(&mut record.response))
            }
            other => extra_response(known_extra(other).and_then(|extra| extra.get_mut("response"))),
        },
        ResponsesAPIStreamingResponse::Generic(record) => {
            extra_response(record.extra.get_mut("response"))
        }
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

// a JSON null stays a value here; the rewrite itself ignores anything that is not an object
fn extra_response(slot: Option<&mut Json>) -> Option<ResponseTarget<'_>> {
    match slot {
        None => None,
        Some(value) => Some(ResponseTarget::Value(value)),
    }
}

#[must_use]
pub fn item_target(event: &mut ResponsesAPIStreamingResponse) -> Option<Target<'_>> {
    match event {
        ResponsesAPIStreamingResponse::Known(known) => match known.as_mut() {
            KnownResponseEvent::OutputItemAdded(record) => match &mut record.item {
                RequiredNullable::Value(extra) => Some(Target::Extra(extra)),
                RequiredNullable::Null => None,
            },
            KnownResponseEvent::OutputItemDone(record) => Some(Target::Extra(&mut record.item)),
            _ => None,
        },
        ResponsesAPIStreamingResponse::Generic(record) => {
            record.extra.get_mut("item").map(Target::Value)
        }
    }
}

#[must_use]
pub fn annotation_target(event: &mut ResponsesAPIStreamingResponse) -> Option<Target<'_>> {
    match event {
        ResponsesAPIStreamingResponse::Known(known) => match known.as_mut() {
            KnownResponseEvent::OutputTextAnnotationAdded(record) => {
                Some(Target::Extra(&mut record.annotation))
            }
            _ => None,
        },
        ResponsesAPIStreamingResponse::Generic(record) => {
            record.extra.get_mut("annotation").map(Target::Value)
        }
    }
}

#[must_use]
pub fn part_target(event: &mut ResponsesAPIStreamingResponse) -> Option<PartTarget<'_>> {
    match event {
        ResponsesAPIStreamingResponse::Known(known) => match known.as_mut() {
            KnownResponseEvent::ContentPartDone(record) => match &mut record.part {
                ContentPartDonePart::OutputText { annotations, .. } => {
                    Some(PartTarget::Extra(annotations))
                }
                ContentPartDonePart::Refusal { .. } | ContentPartDonePart::ReasoningText { .. } => {
                    None
                }
            },
            _ => None,
        },
        ResponsesAPIStreamingResponse::Generic(record) => {
            record.extra.get_mut("part").map(PartTarget::Value)
        }
    }
}

// serde's untagged Record decode reads a float as its private number map and the Dict arm wins, so a JSON value converts by hand
#[must_use]
pub fn record_of(value: &Json) -> Record {
    match value {
        Json::Object(fields) => record_of_fields(fields.iter()),
        Json::List(items) => Record::List(items.iter().map(record_of).collect()),
        scalar @ (Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_)) => {
            Record::Scalar(scalar.clone())
        }
    }
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
        Record::Dict(dict) => Json::Object(
            dict.values
                .iter()
                .map(|(name, child)| (name.clone(), json_of(child)))
                .collect(),
        ),
        Record::Object { object } => Json::Object(
            object
                .fields
                .iter()
                .map(|(name, child)| (name.clone(), json_of(child)))
                .collect(),
        ),
        Record::List(items) => Json::List(items.iter().map(json_of).collect()),
        Record::Scalar(value) => value.clone(),
    }
}

#[must_use]
pub fn extra_of(record: &Record) -> Option<Extra> {
    match json_of(record) {
        Json::Object(fields) => Some(fields.into_iter().collect()),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => None,
    }
}
