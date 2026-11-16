use betterlite_core::api::realtime_contracts::events::AllDeltaType;
use betterlite_prompts::arguments::json::Json;

use super::delta::content_delta_event;
use super::done::{additional_done_events, content_done_event};
use super::error::ContentError;
use super::events::ContentEvent;
use super::ids::ContentIds;
use super::map::ModalityEvent;
use super::start::{StartContent, start_content_events};
use super::state::ContentState;

pub fn handle_openai_modality_event(
    state: &mut ContentState,
    event: ModalityEvent,
    frame: &Json,
    setup_request: Option<&str>,
    ids: &mut dyn ContentIds,
) -> Result<Vec<ContentEvent>, ContentError> {
    let delta_type = event.delta_type();
    let events = match event {
        ModalityEvent::TextDelta | ModalityEvent::AudioDelta => {
            delta_events(state, delta_type, frame, setup_request, ids)?
        }
        ModalityEvent::TextDone | ModalityEvent::AudioDone => done_events(state, delta_type, ids)?,
    };
    state.delta_type = Some(delta_type);
    Ok(events)
}

fn delta_events(
    state: &mut ContentState,
    delta_type: AllDeltaType,
    frame: &Json,
    setup_request: Option<&str>,
    ids: &mut dyn ContentIds,
) -> Result<Vec<ContentEvent>, ContentError> {
    let response_id = state
        .response_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .map_or_else(|| format!("resp_{}", ids.next_id()), str::to_owned);
    let (output_item_id, conversation_id, mut events) =
        new_content(state, &response_id, setup_request, delta_type, ids)?;
    let server_content = frame
        .as_object()
        .and_then(|fields| fields.get("serverContent"))
        .ok_or(ContentError::MissingServerContent)?;
    events.push(content_delta_event(
        server_content,
        &output_item_id,
        &response_id,
        delta_type,
        ids,
    )?);
    state.output_item_id = Some(output_item_id);
    state.response_id = Some(response_id);
    state.conversation_id = conversation_id;
    Ok(events)
}

fn new_content(
    state: &ContentState,
    response_id: &str,
    setup_request: Option<&str>,
    delta_type: AllDeltaType,
    ids: &mut dyn ContentIds,
) -> Result<(String, Option<String>, Vec<ContentEvent>), ContentError> {
    let existing_item = state.output_item_id.as_deref().filter(|id| !id.is_empty());
    let Some(item_id) = existing_item else {
        let generated_item = format!("item_{}", ids.next_id());
        let conversation_id = state
            .conversation_id
            .as_deref()
            .filter(|id| !id.is_empty())
            .map_or_else(|| format!("conv_{}", ids.next_id()), str::to_owned);
        let events = start_content_events(
            &StartContent {
                setup_request,
                response_id,
                output_item_id: &generated_item,
                conversation_id: &conversation_id,
                delta_type,
            },
            ids,
        )?;
        return Ok((generated_item, Some(conversation_id), events));
    };
    Ok((
        item_id.to_owned(),
        state.conversation_id.clone(),
        Vec::new(),
    ))
}

fn done_events(
    state: &ContentState,
    delta_type: AllDeltaType,
    ids: &mut dyn ContentIds,
) -> Result<Vec<ContentEvent>, ContentError> {
    let done = content_done_event(
        state.delta_chunks.as_deref(),
        state.output_item_id.as_deref(),
        state.response_id.as_deref(),
        delta_type,
        ids,
    )?;
    let additional = additional_done_events(&done, ids);
    let mut events = vec![done.event];
    events.extend(additional);
    Ok(events)
}
