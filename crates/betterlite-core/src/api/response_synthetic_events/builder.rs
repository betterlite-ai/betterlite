use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::event_types::{
    Extra, OutputItemAddedEvent, OutputItemDoneEvent, RequiredNullable, ResponseEvent,
};
use betterlite_openai::api::responses_event_records::records::{
    KnownResponseEvent, ResponsesAPIStreamingResponse,
};
use betterlite_openai::api::responses_objects::{ResponseOutputItem, ResponsesApiResponse};
use std::mem;

use super::chunks::{EventBudget, payload_bytes};
use super::error::SyntheticEventsError;
use super::message_events::message_events;
use super::model_dump::{ResponseObject, dump_response_object};
use super::python_value::{has_content, text_or_empty};
use super::reasoning_events::reasoning_events;
use super::stamping::{self, ResponsesCostCalculator};
use super::tool_events::function_call_events;

macro_rules! push_event {
    ($events:expr, $event:expr $(,)?) => {
        $events.push(ResponsesAPIStreamingResponse::Known(Box::new($event)));
    };
}

pub(super) use push_event;

pub(super) struct BuildContext {
    pub(super) chunk_size: i64,
    pub(super) sequence_number: i64,
    pub(super) budget: EventBudget,
}

pub(super) struct ItemContext<'item> {
    pub(super) item_id: &'item str,
    pub(super) output_index: i64,
}

pub(super) struct PartContext<'part> {
    pub(super) item_id: &'part str,
    pub(super) output_index: i64,
    pub(super) content_index: i64,
}

// ast-grep-ignore: no-long-names
pub fn build_synthetic_response_events<C>(
    transformed: &mut ResponsesApiResponse,
    calculator: Option<&C>,
    chunk_size: i64,
) -> Result<Vec<ResponsesAPIStreamingResponse>, SyntheticEventsError>
where
    C: ResponsesCostCalculator,
{
    stamping::stamp_usage_cost(transformed, calculator);
    let mut build = BuildContext {
        chunk_size,
        sequence_number: 0,
        budget: EventBudget::default(),
    };
    // take the output out first: serializing the response would clone it through its field adapter
    let output = mem::take(&mut transformed.output);
    let envelope = payload_bytes::<ResponsesApiResponse>(transformed);
    if let Err(error) = build.budget.charge(3, envelope.saturating_mul(3)) {
        transformed.output = output;
        return Err(error);
    }
    let mut in_progress = transformed.clone();
    transformed.output = output;
    let mut events = Vec::new();
    in_progress.status = Some("in_progress".to_owned());
    push_event!(
        &mut events,
        KnownResponseEvent::ResponseCreated(ResponseEvent {
            response: in_progress.clone(),
            extra: Extra::new(),
        }),
    );
    push_event!(
        &mut events,
        KnownResponseEvent::ResponseInProgress(ResponseEvent {
            response: in_progress,
            extra: Extra::new(),
        }),
    );
    for (position, item) in transformed.output.iter().enumerate() {
        let output_index = i64::try_from(position).unwrap_or(i64::MAX);
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

fn push_item_events(
    events: &mut Vec<ResponsesAPIStreamingResponse>,
    transformed_id: &str,
    item: &ResponseOutputItem,
    output_index: i64,
    build: &mut BuildContext,
) -> Result<(), SyntheticEventsError> {
    build
        .budget
        .charge(3, payload_bytes(item).saturating_mul(3))?;
    let item_payload = dump_response_object(ResponseObject::Model(item))?;
    let item_id = match item_payload.get("id") {
        Some(value) if has_content(value) => text_or_empty(Some(value)),
        _ => transformed_id.to_owned(),
    };
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
    if let Some(Json::Text(kind)) = item_payload.get("type") {
        match kind.as_str() {
            "message" => message_events(events, &context, &item_payload, build)?,
            "function_call" => function_call_events(events, &context, &item_payload, build)?,
            "reasoning" => reasoning_events(events, &context, &item_payload, build)?,
            _ => {}
        }
    }
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
