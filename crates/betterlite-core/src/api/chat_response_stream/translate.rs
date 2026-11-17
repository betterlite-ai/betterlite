use std::collections::VecDeque;

use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::event_types::{
    Extra, OutputTextAnnotationAddedEvent, OutputTextDeltaEvent, ReasoningSummaryTextDeltaEvent,
};
use betterlite_openai::api::responses_event_records::records::KnownResponseEvent;
use indexmap::IndexMap;
use serde_json::Number;

use crate::api::chat_response_stream_items::done::{ReasoningDone, reasoning_done_event};
use crate::api::chat_response_stream_items::error::StreamItemsError;
use crate::api::chat_response_stream_items::items::queue_message_added;
use crate::api::chat_response_stream_items::sources::EventSources;
use crate::api::chat_response_stream_tools::error::StreamToolsError;
use crate::api::chat_response_stream_tools::events::output_item_event;
use crate::api::chat_response_stream_tools::items::has_signed_thinking_block;
use crate::api::chat_response_stream_tools::reasoning::reasoning_ended;
use crate::api::chat_response_stream_tools::state::{
    ChatStreamState, StreamProgress, next_sequence,
};
use crate::api::chat_response_stream_tools::tool_calls::queue_tool_delta_events;
use crate::api::responses_output_details::citation_fields;
use crate::api::responses_tool_namespaces::values::has_value;

use super::chunk;
use super::error::ChatResponseStreamError;

pub(crate) fn ensure_output_item_for_chunk<Context>(
    state: &mut ChatStreamState<Context>,
    sources: &EventSources,
    chunk: &Json,
) -> Result<(), ChatResponseStreamError> {
    if state.progress.sent_output_item_added || chunk::first_choice(chunk).is_none() {
        return Ok(());
    }
    let sequence = next_sequence(state)?;
    state.progress.sent_output_item_added = true;
    let delta = chunk::delta(chunk);
    if starts_reasoning(chunk) {
        state.progress.reasoning_active = true;
        let item_id = reasoning_item_id(&mut state.progress, sources);
        let event = output_item_event(0, reasoning_item(&item_id), sequence, false);
        state.progress.pending_response_events.push(event);
        return Ok(());
    }
    let tool_calls = delta.and_then(|fields| fields.get("tool_calls"));
    if tool_calls.is_some_and(has_value) {
        return Ok(());
    }
    queue_message_added(state, sources)?;
    Ok(())
}

// Only the async iterator closes a reasoning segment, and the close clears the active flag itself
pub(crate) fn close_reasoning_segment<Context>(
    state: &mut ChatStreamState<Context>,
    sources: &EventSources,
    chunk: &Json,
) -> Result<(), ChatResponseStreamError> {
    if !state.progress.reasoning_active {
        return Ok(());
    }
    let delta = chunk::delta(chunk);
    if let Some(text) = delta_text(delta, "reasoning_content")? {
        state.progress.reasoning_content_parts.push(text.to_owned());
    }
    if !reasoning_ended(chunk) {
        return Ok(());
    }
    let content = state.progress.reasoning_content_parts.concat();
    let item_id = reasoning_item_id(&mut state.progress, sources);
    for kind in [
        ReasoningDone::Text,
        ReasoningDone::Part,
        ReasoningDone::Item,
    ] {
        let sequence = next_sequence(state)?;
        let event = reasoning_done_event(&item_id, &content, sequence, kind);
        state.progress.pending_response_events.push(event);
    }
    state.progress.reasoning_active = false;
    Ok(())
}

// LiteLLM's priority: reasoning, text, tool deltas, queued annotations, then tool events queued by an earlier chunk
pub(crate) fn transform<Context>(
    state: &mut ChatStreamState<Context>,
    sources: &EventSources,
    chunk: &Json,
    annotations: &mut VecDeque<KnownResponseEvent>,
) -> Result<Option<KnownResponseEvent>, ChatResponseStreamError> {
    let item_id = generated_id(&mut state.progress.cached_item_id, "msg", sources);
    let delta = chunk::delta(chunk);
    queue_first_annotations(state, &item_id, delta, annotations)?;
    if let Some(reasoning) = delta_text(delta, "reasoning_content")? {
        let rs_id = generated_id(&mut state.progress.cached_reasoning_item_id, "rs", sources);
        return Ok(Some(KnownResponseEvent::ReasoningSummaryTextDelta(
            ReasoningSummaryTextDeltaEvent {
                item_id: rs_id,
                output_index: 0,
                summary_index: 0,
                delta: reasoning.to_owned(),
                extra: Extra::new(),
            },
        )));
    }
    if let Some(content) = delta_text(delta, "content")? {
        if !state.progress.sent_message_item_added {
            queue_message_added(state, sources)?;
        }
        return text_delta_event(state, &item_id, content).map(Some);
    }
    if let Some(tool_calls) = delta
        .and_then(|fields| fields.get("tool_calls"))
        .filter(|value| has_value(value))
    {
        queue_tool_delta_events(state, tool_calls)?;
        if let Some(event) = pop_tool_event(state) {
            return Ok(Some(event));
        }
    }
    Ok(annotations.pop_front().or_else(|| pop_tool_event(state)))
}

fn starts_reasoning(chunk: &Json) -> bool {
    let Some(fields) = chunk::delta(chunk) else {
        return false;
    };
    fields.get("reasoning_content").is_some_and(has_value)
        || (fields.contains_key("thinking_blocks")
            && has_signed_thinking_block(&Json::Object(fields.clone())))
}

// A delta field the declared types say is text; absent, null and falsy values read as no text
fn delta_text<'chunk>(
    delta: Option<&'chunk IndexMap<String, Json>>,
    field: &'static str,
) -> Result<Option<&'chunk str>, ChatResponseStreamError> {
    match delta.and_then(|fields| fields.get(field)) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok((!text.is_empty()).then_some(text.as_str())),
        Some(value) if !has_value(value) => Ok(None),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(ChatResponseStreamError::FieldNotText { field })
        }
    }
}

fn generated_id(slot: &mut Option<String>, prefix: &str, sources: &EventSources) -> String {
    slot.get_or_insert_with(|| format!("{prefix}_{}", (sources.ids)()))
        .clone()
}

fn reasoning_item_id(progress: &mut StreamProgress, sources: &EventSources) -> String {
    let item_id = generated_id(&mut progress.cached_reasoning_item_id, "rs", sources);
    progress.reasoning_item_id = Some(item_id.clone());
    item_id
}

fn reasoning_item(item_id: &str) -> Extra {
    Extra::from([
        ("id".to_owned(), Json::Text(item_id.to_owned())),
        ("type".to_owned(), Json::Text("reasoning".to_owned())),
        ("status".to_owned(), Json::Text("in_progress".to_owned())),
        ("summary".to_owned(), Json::Null),
    ])
}

fn text_delta_event<Context>(
    state: &mut ChatStreamState<Context>,
    item_id: &str,
    content: &str,
) -> Result<KnownResponseEvent, ChatResponseStreamError> {
    let sequence = next_sequence(state)?;
    let number = Json::Number(Number::from(sequence));
    Ok(KnownResponseEvent::OutputTextDelta(OutputTextDeltaEvent {
        item_id: item_id.to_owned(),
        output_index: state.progress.message_output_index,
        content_index: 0,
        delta: content.to_owned(),
        extra: Extra::from([("sequence_number".to_owned(), number)]),
    }))
}

fn pop_tool_event<Context>(state: &mut ChatStreamState<Context>) -> Option<KnownResponseEvent> {
    let queue = &mut state.progress.pending_tool_events;
    if queue.is_empty() {
        return None;
    }
    Some(queue.remove(0).event)
}

fn queue_first_annotations<Context>(
    state: &mut ChatStreamState<Context>,
    item_id: &str,
    delta: Option<&IndexMap<String, Json>>,
    queue: &mut VecDeque<KnownResponseEvent>,
) -> Result<(), ChatResponseStreamError> {
    if state.progress.sent_annotation_events {
        return Ok(());
    }
    let items = match delta.and_then(|fields| fields.get("annotations")) {
        None | Some(Json::Null) => return Ok(()),
        Some(Json::List(items)) if items.is_empty() => return Ok(()),
        Some(Json::List(items)) => items,
        Some(value) if !has_value(value) => return Ok(()),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::Object(_)) => {
            return Err(ChatResponseStreamError::Items(
                StreamItemsError::AnnotationsNotList,
            ));
        }
    };
    state.progress.sent_annotation_events = true;
    let mut parsed = Vec::with_capacity(items.len());
    for item in items {
        let encoded = serde_json::to_string(item)?;
        let annotation = serde_json::from_str(&encoded)
            .map_err(|source| StreamItemsError::Annotation { source })?;
        parsed.push(annotation);
    }
    for (index, annotation) in citation_fields(Some(&parsed)).into_iter().enumerate() {
        let encoded = serde_json::to_string(&annotation)?;
        let annotation_index =
            i64::try_from(index).map_err(|_error| StreamToolsError::IndexOverflow)?;
        let event = OutputTextAnnotationAddedEvent {
            item_id: item_id.to_owned(),
            output_index: state.progress.message_output_index,
            content_index: 0,
            annotation_index,
            annotation: serde_json::from_str(&encoded)?,
            extra: Extra::new(),
        };
        queue.push_back(KnownResponseEvent::OutputTextAnnotationAdded(event));
    }
    Ok(())
}
