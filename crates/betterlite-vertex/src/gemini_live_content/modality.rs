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

struct ResolvedIds {
    output_item_id: Option<String>,
    response_id: Option<String>,
    conversation_id: Option<String>,
}

pub fn handle_openai_modality_event(
    state: &mut ContentState,
    event: ModalityEvent,
    frame: &Json,
    setup_request: Option<&str>,
    ids: &mut dyn ContentIds,
) -> Result<Vec<ContentEvent>, ContentError> {
    let delta_type = event.delta_type();
    let (events, resolved) = match event {
        ModalityEvent::TextDelta | ModalityEvent::AudioDelta => {
            delta_events(state, delta_type, frame, setup_request, ids)?
        }
        ModalityEvent::TextDone | ModalityEvent::AudioDone => done_events(state, delta_type, ids)?,
    };
    state.output_item_id = resolved.output_item_id;
    state.response_id = resolved.response_id;
    state.conversation_id = resolved.conversation_id;
    state.delta_type = Some(delta_type);
    Ok(events)
}

fn delta_events(
    state: &ContentState,
    delta_type: AllDeltaType,
    frame: &Json,
    setup_request: Option<&str>,
    ids: &mut dyn ContentIds,
) -> Result<(Vec<ContentEvent>, ResolvedIds), ContentError> {
    let mut resolved = ResolvedIds {
        output_item_id: state.output_item_id.clone(),
        response_id: state.response_id.clone(),
        conversation_id: state.conversation_id.clone(),
    };
    if resolved.response_id.as_deref().is_none_or(str::is_empty) {
        resolved.response_id = Some(format!("resp_{}", ids.next_id()));
    }
    let mut events = new_content_start(&mut resolved, setup_request, delta_type, ids)?;
    let server_content = frame
        .as_object()
        .and_then(|fields| fields.get("serverContent"))
        .ok_or(ContentError::MissingServerContent)?;
    events.push(content_delta_event(
        server_content,
        resolved.output_item_id.as_deref().unwrap_or_default(),
        resolved.response_id.as_deref().unwrap_or_default(),
        delta_type,
        ids,
    )?);
    Ok((events, resolved))
}

fn new_content_start(
    resolved: &mut ResolvedIds,
    setup_request: Option<&str>,
    delta_type: AllDeltaType,
    ids: &mut dyn ContentIds,
) -> Result<Vec<ContentEvent>, ContentError> {
    let mut events = Vec::new();
    if resolved.output_item_id.as_deref().is_none_or(str::is_empty) {
        resolved.output_item_id = Some(format!("item_{}", ids.next_id()));
        if resolved
            .conversation_id
            .as_deref()
            .is_none_or(str::is_empty)
        {
            resolved.conversation_id = Some(format!("conv_{}", ids.next_id()));
        }
        events.extend(start_content_events(
            &StartContent {
                setup_request,
                response_id: resolved.response_id.as_deref().unwrap_or_default(),
                output_item_id: resolved.output_item_id.as_deref().unwrap_or_default(),
                conversation_id: resolved.conversation_id.as_deref().unwrap_or_default(),
                delta_type,
            },
            ids,
        )?);
    }
    Ok(events)
}

fn done_events(
    state: &ContentState,
    delta_type: AllDeltaType,
    ids: &mut dyn ContentIds,
) -> Result<(Vec<ContentEvent>, ResolvedIds), ContentError> {
    let done = content_done_event(
        state.delta_chunks.as_deref(),
        state.output_item_id.as_deref(),
        state.response_id.as_deref(),
        delta_type,
        ids,
    )?;
    let (item_id, response_id) = done.done_ids().unwrap_or_default();
    let additional =
        additional_done_events(&done, Some(item_id), Some(response_id), delta_type, ids);
    let mut events = Vec::new();
    events.push(done);
    events.extend(additional);
    Ok((
        events,
        ResolvedIds {
            output_item_id: state.output_item_id.clone(),
            response_id: state.response_id.clone(),
            conversation_id: state.conversation_id.clone(),
        },
    ))
}
