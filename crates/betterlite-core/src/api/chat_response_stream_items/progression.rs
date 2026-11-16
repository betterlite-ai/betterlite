use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::KnownResponseEvent;

use crate::api::chat_response_stream_tools::state::ChatStreamState;

use super::completion::completion_content;
use super::done::{MessageDone, message_done_event};
use super::error::StreamItemsError;
use super::items::queue_message_added;
use super::lifecycle::{LifecycleKind, create_lifecycle_event};
use super::sources::EventSources;

pub fn return_default_done_events<Context>(
    state: &mut ChatStreamState<Context>,
    sources: &EventSources,
    completion: &Json,
) -> Result<Option<KnownResponseEvent>, StreamItemsError> {
    if state.progress.sent_message_item_added {
        return default_done_event(state, sources, completion);
    }
    let final_content = completion_content(completion)?;
    if final_content.is_empty() {
        state.progress.sent_output_text_done = true;
        state.progress.sent_content_part_done = true;
        state.progress.sent_output_item_done = true;
        return Ok(None);
    }
    queue_message_added(state, sources)?;
    Ok(state.progress.pending_response_events.drain(..1).next())
}

// LiteLLM marks each done flag before it returns the event that flag stands for.
fn default_done_event<Context>(
    state: &mut ChatStreamState<Context>,
    sources: &EventSources,
    completion: &Json,
) -> Result<Option<KnownResponseEvent>, StreamItemsError> {
    let Some(kind) = next_done_kind(state) else {
        return Ok(None);
    };
    match kind {
        MessageDone::Text => state.progress.sent_output_text_done = true,
        MessageDone::ContentPart => state.progress.sent_content_part_done = true,
        MessageDone::Item => state.progress.sent_output_item_done = true,
    }
    Ok(Some(message_done_event(state, sources, completion, kind)?))
}

const fn next_done_kind<Context>(state: &ChatStreamState<Context>) -> Option<MessageDone> {
    if !state.progress.sent_output_text_done {
        return Some(MessageDone::Text);
    }
    if !state.progress.sent_content_part_done {
        return Some(MessageDone::ContentPart);
    }
    if !state.progress.sent_output_item_done {
        return Some(MessageDone::Item);
    }
    None
}

pub fn return_default_initial_events<Context>(
    state: &mut ChatStreamState<Context>,
    sources: &EventSources,
) -> Result<Option<KnownResponseEvent>, StreamItemsError> {
    if !state.progress.sent_response_created {
        state.progress.sent_response_created = true;
        return Ok(Some(create_lifecycle_event(
            state,
            sources,
            LifecycleKind::Created,
        )?));
    }
    if !state.progress.sent_in_progress {
        state.progress.sent_in_progress = true;
        return Ok(Some(create_lifecycle_event(
            state,
            sources,
            LifecycleKind::InProgress,
        )?));
    }
    Ok(None)
}

#[must_use]
pub const fn is_stream_finished<Context>(state: &ChatStreamState<Context>) -> bool {
    state.progress.sent_output_text_done
        && state.progress.sent_content_part_done
        && state.progress.sent_output_item_done
}
