use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::event_types::{
    Extra, OutputItemAddedEvent, OutputItemDoneEvent, RequiredNullable, ResponseEvent,
};
use betterlite_openai::api::responses_event_records::records::{
    KnownResponseEvent, ResponsesAPIStreamingResponse,
};
use betterlite_openai::api::responses_objects::{ResponseOutputItem, ResponsesApiResponse};

use super::error::SyntheticEventsError;
use super::message_events::message_events;
use super::model_dump::{ResponseObject, dump_response_object};
use super::python_value::{text_or_empty, truthy};
use super::reasoning_events::reasoning_events;
use super::stamping::{self, ResponsesCostCalculator};
use super::tool_events::function_call_events;

macro_rules! push_event {
    ($events:expr, $event:expr) => {
        $events.push(ResponsesAPIStreamingResponse::Known(Box::new($event)));
    };
}

pub(super) use push_event;

pub(super) struct BuildContext {
    pub(super) chunk_size: i64,
    pub(super) sequence_number: i64,
}

pub(super) struct ItemContext<'a> {
    pub(super) item_id: &'a str,
    pub(super) output_index: i64,
}

pub(super) struct PartContext<'a> {
    pub(super) item_id: &'a str,
    pub(super) output_index: i64,
    pub(super) content_index: i64,
}

pub fn build_synthetic_response_events<C: ResponsesCostCalculator>(
    transformed: &mut ResponsesApiResponse,
    calculator: Option<&C>,
    chunk_size: i64,
) -> Result<Vec<ResponsesAPIStreamingResponse>, SyntheticEventsError> {
    stamping::stamp_usage_cost(transformed, calculator);
    let mut events = Vec::new();
    push_status_events(&mut events, transformed);
    let mut build = BuildContext {
        chunk_size,
        sequence_number: 0,
    };
    for (output_index, item) in transformed.output.iter().enumerate() {
        let output_index = i64::try_from(output_index).unwrap_or(i64::MAX);
        push_item_events(&mut events, &transformed.id, item, output_index, &mut build)?;
    }
    push_event!(
        &mut events,
        KnownResponseEvent::ResponseCompleted(ResponseEvent {
            response: transformed.clone(),
            extra: Extra::new(),
        }),
    );
    Ok(events)
}

fn push_status_events(
    events: &mut Vec<ResponsesAPIStreamingResponse>,
    transformed: &ResponsesApiResponse,
) {
    let mut in_progress = transformed.clone();
    in_progress.status = Some("in_progress".to_owned());
    in_progress.output = Vec::new();
    push_event!(
        events,
        KnownResponseEvent::ResponseCreated(ResponseEvent {
            response: in_progress.clone(),
            extra: Extra::new(),
        }),
    );
    push_event!(
        events,
        KnownResponseEvent::ResponseInProgress(ResponseEvent {
            response: in_progress,
            extra: Extra::new(),
        }),
    );
}

fn push_item_events(
    events: &mut Vec<ResponsesAPIStreamingResponse>,
    transformed_id: &str,
    item: &ResponseOutputItem,
    output_index: i64,
    build: &mut BuildContext,
) -> Result<(), SyntheticEventsError> {
    let item_payload = dump_response_object(ResponseObject::Model(item))?;
    let item_id = item_id(&item_payload, transformed_id);
    push_event!(
        events,
        KnownResponseEvent::OutputItemAdded(OutputItemAddedEvent {
            output_index,
            item: RequiredNullable::Value(item_payload.clone()),
            extra: Extra::new(),
        }),
    );
    let context = ItemContext {
        item_id: &item_id,
        output_index,
    };
    item_type_events(events, &context, &item_payload, build)?;
    build.sequence_number = build.sequence_number.saturating_add(1);
    push_event!(
        events,
        KnownResponseEvent::OutputItemDone(OutputItemDoneEvent {
            output_index,
            sequence_number: build.sequence_number,
            item: item_payload,
            extra: Extra::new(),
        }),
    );
    Ok(())
}

fn item_id(payload: &Extra, fallback: &str) -> String {
    match payload.get("id") {
        Some(value) if truthy(value) => text_or_empty(Some(value)),
        _ => fallback.to_owned(),
    }
}

fn item_type_events(
    events: &mut Vec<ResponsesAPIStreamingResponse>,
    context: &ItemContext<'_>,
    payload: &Extra,
    build: &mut BuildContext,
) -> Result<(), SyntheticEventsError> {
    match payload.get("type") {
        Some(Json::Text(kind)) if kind == "message" => {
            message_events(events, context, payload, build.chunk_size)
        }
        Some(Json::Text(kind)) if kind == "function_call" => {
            function_call_events(events, context, payload, build.chunk_size)
        }
        Some(Json::Text(kind)) if kind == "reasoning" => {
            reasoning_events(events, context, payload, build)
        }
        _ => Ok(()),
    }
}
