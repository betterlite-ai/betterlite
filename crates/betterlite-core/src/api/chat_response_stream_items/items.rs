use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::event_types::{
    ContentPartAddedEvent, Extra, OutputItemAddedEvent, RequiredNullable,
};
use betterlite_openai::api::responses_event_records::records::KnownResponseEvent;

use crate::api::chat_response_stream_tools::error::StreamToolsError;
use crate::api::chat_response_stream_tools::state::{ChatStreamState, next_sequence};

use super::error::StreamItemsError;
use super::sources::EventSources;
use super::support::numbered;

pub fn queue_message_item_added_events<Context>(
    state: &mut ChatStreamState<Context>,
    sources: &EventSources,
) -> Result<(), StreamItemsError> {
    let item_id = ensure_cached_item_id(state, sources);
    state.progress.sent_message_item_added = true;
    state.progress.sent_content_part_added = true;
    let output_index = if state.progress.cached_reasoning_item_id.is_some() {
        let index = state.progress.next_tool_output_index;
        state.progress.next_tool_output_index = index
            .checked_add(1)
            .ok_or(StreamToolsError::IndexOverflow)?;
        index
    } else {
        0
    };
    state.progress.message_output_index = output_index;
    let sequence_number = next_sequence(state)?;
    state
        .progress
        .pending_response_events
        .push(KnownResponseEvent::OutputItemAdded(OutputItemAddedEvent {
            output_index,
            item: RequiredNullable::Value(message_item(&item_id)),
            extra: numbered(sequence_number),
        }));
    let content_part = create_content_part_added_event(state, sources)?;
    state.progress.pending_response_events.push(content_part);
    Ok(())
}

pub fn create_content_part_added_event<Context>(
    state: &mut ChatStreamState<Context>,
    sources: &EventSources,
) -> Result<KnownResponseEvent, StreamItemsError> {
    let item_id = ensure_cached_item_id(state, sources);
    let sequence_number = next_sequence(state)?;
    Ok(KnownResponseEvent::ContentPartAdded(
        ContentPartAddedEvent {
            item_id,
            output_index: state.progress.message_output_index,
            content_index: 0,
            part: content_part(),
            extra: numbered(sequence_number),
        },
    ))
}

pub(super) fn ensure_cached_item_id<Context>(
    state: &mut ChatStreamState<Context>,
    sources: &EventSources,
) -> String {
    match state.progress.cached_item_id.clone() {
        Some(item_id) => item_id,
        None => {
            let item_id = format!("msg_{}", sources.next_id());
            state.progress.cached_item_id = Some(item_id.clone());
            item_id
        }
    }
}

fn message_item(item_id: &str) -> Extra {
    Extra::from([
        ("id".to_owned(), Json::Text(item_id.to_owned())),
        ("type".to_owned(), Json::Text("message".to_owned())),
        ("status".to_owned(), Json::Text("in_progress".to_owned())),
        ("role".to_owned(), Json::Text("assistant".to_owned())),
        ("content".to_owned(), Json::List(Vec::new())),
    ])
}

fn content_part() -> Extra {
    Extra::from([
        ("type".to_owned(), Json::Text("output_text".to_owned())),
        ("text".to_owned(), Json::Text(String::new())),
        ("annotations".to_owned(), Json::List(Vec::new())),
    ])
}
