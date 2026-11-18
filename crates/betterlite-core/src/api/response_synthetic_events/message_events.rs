use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::event_types::{
    ContentPartAddedEvent, ContentPartDoneEvent, ContentPartDonePart, Extra,
    OutputTextAnnotationAddedEvent, OutputTextDeltaEvent, OutputTextDoneEvent, RefusalDeltaEvent,
    RefusalDoneEvent,
};
use betterlite_openai::api::responses_event_records::records::{
    KnownResponseEvent, ResponsesAPIStreamingResponse,
};
use serde_json::json;

use super::builder::{BuildContext, ItemContext, PartContext, push_event};
use super::chunks::{EventBudget, delta_chunks};
use super::error::SyntheticEventsError;
use super::model_dump::{ResponseObject, dump_response_object};
use super::python_value::{has_content, json_array_or_empty, text_or_empty};

pub(super) fn message_events(
    events: &mut Vec<ResponsesAPIStreamingResponse>,
    context: &ItemContext<'_>,
    payload: &Extra,
    build: &mut BuildContext,
) -> Result<(), SyntheticEventsError> {
    let content_parts = json_array_or_empty(payload.get("content"));
    for (content_index, part) in content_parts.iter().enumerate() {
        let content_index = i64::try_from(content_index).unwrap_or(i64::MAX);
        content_part_events(events, context, part, content_index, build)?;
    }
    Ok(())
}

fn content_part_events(
    events: &mut Vec<ResponsesAPIStreamingResponse>,
    context: &ItemContext<'_>,
    part: &Json,
    content_index: i64,
    build: &mut BuildContext,
) -> Result<(), SyntheticEventsError> {
    let part_payload = dump_response_object(ResponseObject::Value(part))?;
    push_event!(
        events,
        KnownResponseEvent::ContentPartAdded(ContentPartAddedEvent {
            item_id: context.item_id.to_owned(),
            output_index: context.output_index,
            content_index,
            part: part_payload.clone(),
            extra: Extra::new(),
        })
    );
    let part_context = PartContext {
        item_id: context.item_id,
        output_index: context.output_index,
        content_index,
    };
    text_like_part_events(events, &part_context, &part_payload, build)?;
    if let Some(part) = content_part_done_part(&part_payload)? {
        push_event!(
            events,
            KnownResponseEvent::ContentPartDone(ContentPartDoneEvent {
                item_id: context.item_id.to_owned(),
                output_index: context.output_index,
                content_index,
                part,
                extra: Extra::new(),
            })
        );
    }
    Ok(())
}

fn text_like_part_events(
    events: &mut Vec<ResponsesAPIStreamingResponse>,
    context: &PartContext<'_>,
    payload: &Extra,
    build: &mut BuildContext,
) -> Result<(), SyntheticEventsError> {
    let Some(Json::Text(part_type)) = payload.get("type") else {
        return Ok(());
    };
    let kind = if part_type == "output_text" {
        DeltaKind::OutputText
    } else if part_type == "refusal" {
        DeltaKind::Refusal
    } else {
        return Ok(());
    };
    let text = if matches!(kind, DeltaKind::OutputText) {
        text_or_empty(payload.get("text"))
    } else {
        text_or_empty(payload.get("refusal"))
    };
    push_delta_events(events, context, &text, build, kind)?;
    if matches!(kind, DeltaKind::OutputText) {
        annotation_events(
            events,
            context,
            json_array_or_empty(payload.get("annotations")),
            &mut build.budget,
        )?;
    }
    push_done_event(events, context, &text, kind);
    Ok(())
}

#[derive(Clone, Copy)]
enum DeltaKind {
    OutputText,
    Refusal,
}

fn push_delta_events(
    events: &mut Vec<ResponsesAPIStreamingResponse>,
    context: &PartContext<'_>,
    text: &str,
    build: &mut BuildContext,
    kind: DeltaKind,
) -> Result<(), SyntheticEventsError> {
    for delta in delta_chunks(
        text,
        build.chunk_size,
        &mut build.budget,
        context.item_id.len(),
    )? {
        push_event!(
            events,
            match kind {
                DeltaKind::OutputText =>
                    KnownResponseEvent::OutputTextDelta(OutputTextDeltaEvent {
                        item_id: context.item_id.to_owned(),
                        output_index: context.output_index,
                        content_index: context.content_index,
                        delta,
                        extra: Extra::new(),
                    }),
                DeltaKind::Refusal => KnownResponseEvent::RefusalDelta(RefusalDeltaEvent {
                    item_id: context.item_id.to_owned(),
                    output_index: context.output_index,
                    content_index: context.content_index,
                    delta,
                    extra: Extra::new(),
                }),
            }
        );
    }
    Ok(())
}

fn push_done_event(
    events: &mut Vec<ResponsesAPIStreamingResponse>,
    context: &PartContext<'_>,
    text: &str,
    kind: DeltaKind,
) {
    let done_event = match kind {
        DeltaKind::OutputText => KnownResponseEvent::OutputTextDone(OutputTextDoneEvent {
            item_id: context.item_id.to_owned(),
            output_index: context.output_index,
            content_index: context.content_index,
            text: text.to_owned(),
            extra: Extra::new(),
        }),
        DeltaKind::Refusal => KnownResponseEvent::RefusalDone(RefusalDoneEvent {
            item_id: context.item_id.to_owned(),
            output_index: context.output_index,
            content_index: context.content_index,
            refusal: text.to_owned(),
            extra: Extra::new(),
        }),
    };
    push_event!(events, done_event);
}

fn annotation_events(
    events: &mut Vec<ResponsesAPIStreamingResponse>,
    context: &PartContext<'_>,
    annotations: &[Json],
    budget: &mut EventBudget,
) -> Result<(), SyntheticEventsError> {
    budget.charge(
        annotations.len(),
        annotations.len().saturating_mul(context.item_id.len()),
    )?;
    for (annotation_index, annotation) in annotations.iter().enumerate() {
        let annotation_index = i64::try_from(annotation_index).unwrap_or(i64::MAX);
        let Json::Object(fields) = annotation else {
            return Err(SyntheticEventsError::NotAnObject {
                field: "annotation",
            });
        };
        push_event!(
            events,
            KnownResponseEvent::OutputTextAnnotationAdded(OutputTextAnnotationAddedEvent {
                item_id: context.item_id.to_owned(),
                output_index: context.output_index,
                content_index: context.content_index,
                annotation_index,
                annotation: fields.clone().into_iter().collect(),
                extra: Extra::new(),
            })
        );
    }
    Ok(())
}

fn content_part_done_part(
    payload: &Extra,
) -> Result<Option<ContentPartDonePart>, SyntheticEventsError> {
    let part = match payload.get("type") {
        Some(Json::Text(kind)) if kind == "output_text" => {
            let annotations = match payload.get("annotations") {
                Some(value) if has_content(value) => value.clone(),
                _ => Json::List(Vec::new()),
            };
            let logprobs = payload.get("logprobs").cloned().unwrap_or(Json::Null);
            serde_json::from_value(json!({
                "type": "output_text",
                "text": text_or_empty(payload.get("text")),
                "annotations": annotations,
                "logprobs": logprobs,
            }))?
        }
        Some(Json::Text(kind)) if kind == "refusal" => ContentPartDonePart::Refusal {
            refusal: text_or_empty(payload.get("refusal")),
            extra: Extra::new(),
        },
        Some(Json::Text(kind)) if kind == "reasoning_text" => ContentPartDonePart::ReasoningText {
            reasoning: text_or_empty(payload.get("reasoning")),
            extra: Extra::new(),
        },
        _ => return Ok(None),
    };
    Ok(Some(part))
}
