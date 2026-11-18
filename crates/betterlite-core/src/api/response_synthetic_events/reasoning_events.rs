use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::event_types::{
    Extra, ReasoningSummaryPartDoneEvent, ReasoningSummaryTextDeltaEvent,
    ReasoningSummaryTextDoneEvent,
};
use betterlite_openai::api::responses_event_records::records::{
    KnownResponseEvent, ResponsesAPIStreamingResponse,
};

use super::builder::{BuildContext, ItemContext, push_event};
use super::chunks::delta_chunks;
use super::error::SyntheticEventsError;
use super::model_dump::{ResponseObject, dump_response_object};
use super::python_value::{json_array_or_empty, text_or_empty};

pub(super) fn reasoning_events(
    events: &mut Vec<ResponsesAPIStreamingResponse>,
    context: &ItemContext<'_>,
    payload: &Extra,
    build: &mut BuildContext,
) -> Result<(), SyntheticEventsError> {
    let summaries = json_array_or_empty(payload.get("summary"));
    for (summary_index, summary) in summaries.iter().enumerate() {
        let summary_index = i64::try_from(summary_index).unwrap_or(i64::MAX);
        reasoning_summary_events(events, context, summary, summary_index, build)?;
    }
    Ok(())
}

fn reasoning_summary_events(
    events: &mut Vec<ResponsesAPIStreamingResponse>,
    context: &ItemContext<'_>,
    summary: &Json,
    summary_index: i64,
    build: &mut BuildContext,
) -> Result<(), SyntheticEventsError> {
    let summary_payload = dump_response_object(ResponseObject::Value(summary))?;
    let text = text_or_empty(summary_payload.get("text"));
    let chunk_size = build.chunk_size;
    for delta in delta_chunks(&text, chunk_size, &mut build.budget, context.item_id.len())? {
        push_event!(
            events,
            KnownResponseEvent::ReasoningSummaryTextDelta(ReasoningSummaryTextDeltaEvent {
                item_id: context.item_id.to_owned(),
                output_index: context.output_index,
                summary_index,
                delta,
                extra: Extra::new(),
            })
        );
    }
    let bytes = context.item_id.len().saturating_mul(2);
    build.budget.charge(2, bytes.saturating_add(text.len()))?;
    build.sequence_number = build.sequence_number.saturating_add(1);
    push_event!(
        events,
        KnownResponseEvent::ReasoningSummaryTextDone(ReasoningSummaryTextDoneEvent {
            item_id: context.item_id.to_owned(),
            output_index: context.output_index,
            sequence_number: build.sequence_number,
            summary_index,
            text,
            extra: Extra::new(),
        })
    );
    build.sequence_number = build.sequence_number.saturating_add(1);
    push_event!(
        events,
        KnownResponseEvent::ReasoningSummaryPartDone(ReasoningSummaryPartDoneEvent {
            item_id: context.item_id.to_owned(),
            output_index: context.output_index,
            sequence_number: build.sequence_number,
            summary_index,
            part: summary_payload,
            extra: Extra::new(),
        })
    );
    Ok(())
}
