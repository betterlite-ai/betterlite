use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_event_records::records::KnownResponseEvent;

use crate::api::chat_response_stream_tools::state::ChatStreamState;

use super::completion::completion_content;
use super::done::{
    create_output_content_part_done_event, create_output_item_done_event,
    create_output_text_done_event,
};
use super::error::StreamItemsError;
use super::items::queue_message_item_added_events;
use super::lifecycle::{create_response_created_event, create_response_in_progress_event};
use super::sources::EventSources;

pub fn return_default_done_events<Context>(
    state: &mut ChatStreamState<Context>,
    sources: &EventSources,
    completion: &Json,
) -> Result<Option<KnownResponseEvent>, StreamItemsError> {
    if !state.progress.sent_message_item_added {
        let final_content = completion_content(completion)?;
        if final_content.is_empty() {
            state.progress.sent_output_text_done = true;
            state.progress.sent_content_part_done = true;
            state.progress.sent_output_item_done = true;
            return Ok(None);
        }
        queue_message_item_added_events(state, sources)?;
        return Ok(state.progress.pending_response_events.drain(..1).next());
    }
    if !state.progress.sent_output_text_done {
        state.progress.sent_output_text_done = true;
        return Ok(Some(create_output_text_done_event(
            state, sources, completion,
        )?));
    }
    if !state.progress.sent_content_part_done {
        state.progress.sent_content_part_done = true;
        return Ok(Some(create_output_content_part_done_event(
            state, sources, completion,
        )?));
    }
    if !state.progress.sent_output_item_done {
        state.progress.sent_output_item_done = true;
        return Ok(Some(create_output_item_done_event(
            state, sources, completion,
        )?));
    }
    Ok(None)
}

pub fn return_default_initial_events<Context>(
    state: &mut ChatStreamState<Context>,
    sources: &EventSources,
) -> Result<Option<KnownResponseEvent>, StreamItemsError> {
    if !state.progress.sent_response_created {
        state.progress.sent_response_created = true;
        return Ok(Some(create_response_created_event(state, sources)?));
    }
    if !state.progress.sent_in_progress {
        state.progress.sent_in_progress = true;
        return Ok(Some(create_response_in_progress_event(state, sources)?));
    }
    Ok(None)
}

#[must_use]
pub const fn is_stream_finished<Context>(state: &ChatStreamState<Context>) -> bool {
    state.progress.sent_output_text_done
        && state.progress.sent_content_part_done
        && state.progress.sent_output_item_done
}
