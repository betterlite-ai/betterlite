use betterlite_core::api::realtime_contracts::events::AllDeltaType;
use betterlite_prompts::arguments::json::Json;
use uuid::Uuid;

use super::error::LiveFrameError;
use super::events::{
    INPUT_TRANSCRIPT_COMPLETED, InputTranscriptEvent, OUTPUT_TRANSCRIPT_DELTA,
    OutputTranscriptEvent, event_json,
};
use super::ids::DrawnIds;
use super::transform::FrameState;
use crate::gemini_live_accounting::transcription::consume_transcript_estimate;
use crate::gemini_live_client::records::Transcription;
use crate::gemini_live_content::start::{StartContent, start_content_events};

const MODEL_CONTENT_KEYS: [&str; 4] = [
    "modelTurn",
    "turnComplete",
    "interrupted",
    "generationComplete",
];

pub(super) fn transcribe(
    work: &mut FrameState<'_>,
    uuids: &mut dyn FnMut() -> Uuid,
) -> Result<(), LiveFrameError> {
    let Some(content) = work.frame.message.server_content.clone() else {
        return Ok(());
    };
    if let Some(text) = transcript_text(content.input_transcription.as_ref()) {
        let usage =
            consume_transcript_estimate(work.session, work.context.catalog, work.context.model)?;
        let event = InputTranscriptEvent {
            event_type: INPUT_TRANSCRIPT_COMPLETED,
            event_id: format!("event_{}", uuids()),
            transcript: text,
            item_id: format!("item_{}", uuids()),
            content_index: 0,
            usage,
        };
        work.events.push(event_json(&event)?);
    }
    if let Some(text) = transcript_text(content.output_transcription.as_ref()) {
        push_output_transcript(work, &text, uuids)?;
    }
    Ok(())
}

pub(super) fn transcription_only_content(work: &mut FrameState<'_>) -> bool {
    let Some(Json::Object(content)) = work.frame.entries.get_mut("serverContent") else {
        return false;
    };
    if work.caller.delta_type.is_none() && !content.contains_key("modelTurn") {
        drop(content.shift_remove("generationComplete"));
    }
    !MODEL_CONTENT_KEYS
        .iter()
        .any(|key| content.contains_key(*key))
}

fn push_output_transcript(
    work: &mut FrameState<'_>,
    text: &str,
    uuids: &mut dyn FnMut() -> Uuid,
) -> Result<(), LiveFrameError> {
    let response_id = ensure_response_id(work, uuids);
    let item_id = match work.caller.output_item_id.as_deref() {
        Some(existing) => existing.to_owned(),
        None => start_audio_item(work, &response_id, uuids)?,
    };
    let event = OutputTranscriptEvent {
        event_type: OUTPUT_TRANSCRIPT_DELTA,
        event_id: format!("event_{}", uuids()),
        transcript: text.to_owned(),
        item_id,
        content_index: 0,
        output_index: 0,
        response_id,
        delta: text.to_owned(),
    };
    work.events.push(event_json(&event)?);
    Ok(())
}

fn ensure_response_id(work: &mut FrameState<'_>, uuids: &mut dyn FnMut() -> Uuid) -> String {
    if let Some(existing) = work.caller.response_id.as_deref() {
        return existing.to_owned();
    }
    let generated = format!("resp_{}", uuids());
    work.caller.response_id = Some(generated.clone());
    generated
}

fn start_audio_item(
    work: &mut FrameState<'_>,
    response_id: &str,
    uuids: &mut dyn FnMut() -> Uuid,
) -> Result<String, LiveFrameError> {
    let item_id = format!("item_{}", uuids());
    let conversation_id = work
        .caller
        .conversation_id
        .as_deref()
        .filter(|existing| !existing.is_empty())
        .map_or_else(|| format!("conv_{}", uuids()), str::to_owned);
    let events = start_content_events(
        &StartContent {
            setup_request: work.caller.session_configuration_request.as_deref(),
            response_id,
            output_item_id: &item_id,
            conversation_id: &conversation_id,
            delta_type: AllDeltaType::Audio,
        },
        &mut DrawnIds(uuids),
    )?;
    for event in &events {
        work.events.push(event_json(event)?);
    }
    work.caller.conversation_id = Some(conversation_id);
    work.caller.output_item_id = Some(item_id.clone());
    Ok(item_id)
}

fn transcript_text(transcription: Option<&Transcription>) -> Option<String> {
    transcription
        .and_then(|event| event.text.as_deref())
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}
