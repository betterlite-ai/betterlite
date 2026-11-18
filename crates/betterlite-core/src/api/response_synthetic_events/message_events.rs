use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::event_types::{
    ContentPartAddedEvent, ContentPartDoneEvent, ContentPartDonePart, Extra,
    OutputTextAnnotationAddedEvent, OutputTextDeltaEvent, OutputTextDoneEvent, RefusalDeltaEvent,
    RefusalDoneEvent,
};
use betterlite_openai::api::responses_event_records::records::{
    KnownResponseEvent, ResponsesAPIStreamingResponse,
};
use indexmap::IndexMap;

use super::builder::{ItemContext, PartContext, push_event};
use super::chunks::delta_chunks;
use super::error::SyntheticEventsError;
use super::json_value;
use super::model_dump::{ResponseObject, dump_response_object};
use super::python_value::{text_or_empty, truthy};

pub(super) fn message_events(
    events: &mut Vec<ResponsesAPIStreamingResponse>,
    context: &ItemContext<'_>,
    payload: &Extra,
    chunk_size: i64,
) -> Result<(), SyntheticEventsError> {
    let content_parts: &[Json] = match payload.get("content") {
        Some(Json::List(items)) => items,
        _ => &[],
    };
    for (content_index, part) in content_parts.iter().enumerate() {
        let content_index = i64::try_from(content_index).unwrap_or(i64::MAX);
        content_part_events(events, context, part, content_index, chunk_size)?;
    }
    Ok(())
}

fn content_part_events(
    events: &mut Vec<ResponsesAPIStreamingResponse>,
    context: &ItemContext<'_>,
    part: &Json,
    content_index: i64,
    chunk_size: i64,
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
    text_like_part_events(events, &part_context, &part_payload, chunk_size)?;
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
    chunk_size: i64,
) -> Result<(), SyntheticEventsError> {
    match payload.get("type") {
        Some(Json::Text(kind)) if kind == "output_text" => {
            output_text_part_events(events, context, payload, chunk_size)
        }
        Some(Json::Text(kind)) if kind == "refusal" => {
            refusal_part_events(events, context, payload, chunk_size)
        }
        _ => Ok(()),
    }
}

fn output_text_part_events(
    events: &mut Vec<ResponsesAPIStreamingResponse>,
    context: &PartContext<'_>,
    payload: &Extra,
    chunk_size: i64,
) -> Result<(), SyntheticEventsError> {
    let text = text_or_empty(payload.get("text"));
    for delta in delta_chunks(&text, chunk_size)? {
        push_event!(
            events,
            KnownResponseEvent::OutputTextDelta(OutputTextDeltaEvent {
                item_id: context.item_id.to_owned(),
                output_index: context.output_index,
                content_index: context.content_index,
                delta,
                extra: Extra::new(),
            })
        );
    }
    let annotations: &[Json] = match payload.get("annotations") {
        Some(Json::List(items)) => items,
        _ => &[],
    };
    annotation_events(events, context, annotations)?;
    push_event!(
        events,
        KnownResponseEvent::OutputTextDone(OutputTextDoneEvent {
            item_id: context.item_id.to_owned(),
            output_index: context.output_index,
            content_index: context.content_index,
            text,
            extra: Extra::new(),
        })
    );
    Ok(())
}

fn annotation_events(
    events: &mut Vec<ResponsesAPIStreamingResponse>,
    context: &PartContext<'_>,
    annotations: &[Json],
) -> Result<(), SyntheticEventsError> {
    for (annotation_index, annotation) in annotations.iter().enumerate() {
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
                annotation_index: i64::try_from(annotation_index).unwrap_or(i64::MAX),
                annotation: fields.clone().into_iter().collect(),
                extra: Extra::new(),
            })
        );
    }
    Ok(())
}

fn refusal_part_events(
    events: &mut Vec<ResponsesAPIStreamingResponse>,
    context: &PartContext<'_>,
    payload: &Extra,
    chunk_size: i64,
) -> Result<(), SyntheticEventsError> {
    let refusal = text_or_empty(payload.get("refusal"));
    for delta in delta_chunks(&refusal, chunk_size)? {
        push_event!(
            events,
            KnownResponseEvent::RefusalDelta(RefusalDeltaEvent {
                item_id: context.item_id.to_owned(),
                output_index: context.output_index,
                content_index: context.content_index,
                delta,
                extra: Extra::new(),
            })
        );
    }
    push_event!(
        events,
        KnownResponseEvent::RefusalDone(RefusalDoneEvent {
            item_id: context.item_id.to_owned(),
            output_index: context.output_index,
            content_index: context.content_index,
            refusal,
            extra: Extra::new(),
        })
    );
    Ok(())
}

fn content_part_done_part(
    payload: &Extra,
) -> Result<Option<ContentPartDonePart>, SyntheticEventsError> {
    let part = match payload.get("type") {
        Some(Json::Text(kind)) if kind == "output_text" => output_text_done_part(payload)?,
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

fn output_text_done_part(payload: &Extra) -> Result<ContentPartDonePart, SyntheticEventsError> {
    let annotations = match payload.get("annotations") {
        Some(value) if truthy(value) => value.clone(),
        _ => Json::List(Vec::new()),
    };
    let logprobs = payload.get("logprobs").cloned().unwrap_or(Json::Null);
    let mut part = IndexMap::new();
    part.insert("type".to_owned(), Json::Text("output_text".to_owned()));
    part.insert(
        "text".to_owned(),
        Json::Text(text_or_empty(payload.get("text"))),
    );
    part.insert("annotations".to_owned(), annotations);
    part.insert("logprobs".to_owned(), logprobs);
    json_value::from_json(&Json::Object(part))
}
