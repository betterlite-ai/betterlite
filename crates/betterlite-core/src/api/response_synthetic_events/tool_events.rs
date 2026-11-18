use betterlite_openai::api::responses_event_records::event_types::{
    Extra, FunctionArgsDelta, FunctionArgsDone,
};
use betterlite_openai::api::responses_event_records::records::{
    KnownResponseEvent, ResponsesAPIStreamingResponse,
};

use super::builder::{BuildContext, ItemContext, push_event};
use super::chunks::{delta_chunks, payload_bytes};
use super::error::SyntheticEventsError;
use super::python_value::text_or_empty;

pub(super) fn function_call_events(
    events: &mut Vec<ResponsesAPIStreamingResponse>,
    context: &ItemContext<'_>,
    payload: &Extra,
    build: &mut BuildContext,
) -> Result<(), SyntheticEventsError> {
    let copied = payload_bytes(&payload.get("arguments"));
    build.budget.charge(1, copied)?;
    let arguments = text_or_empty(payload.get("arguments"));
    let chunk_size = build.chunk_size;
    for delta in delta_chunks(
        &arguments,
        chunk_size,
        &mut build.budget,
        context.item_id.len(),
    )? {
        push_event!(
            events,
            KnownResponseEvent::FunctionCallArgumentsDelta(FunctionArgsDelta {
                item_id: context.item_id.to_owned(),
                output_index: context.output_index,
                delta,
                extra: Extra::new(),
            })
        );
    }
    push_event!(
        events,
        KnownResponseEvent::FunctionCallArgumentsDone(FunctionArgsDone {
            item_id: context.item_id.to_owned(),
            output_index: context.output_index,
            arguments,
            extra: Extra::new(),
        })
    );
    Ok(())
}
