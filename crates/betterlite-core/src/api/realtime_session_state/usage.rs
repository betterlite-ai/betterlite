use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::RealtimeStreaming;
use super::TRANSCRIPTION_COMPLETED;
use super::error::SessionStateError;
use super::store::SessionMessage;
use crate::api::realtime_contracts::events::AudioTranscriptionUsage;

// Only the type and usage are kept: the transcript already sits in input_messages, and an event the filter logs needs no second copy
pub(crate) fn capture_transcription_usage(
    state: &mut RealtimeStreaming,
    event: &IndexMap<String, Json>,
) {
    let Some(usage) = event.get("usage") else {
        return;
    };
    if matches!(usage, Json::Null) || state.should_store_message(event) {
        return;
    }
    state.messages.push(IndexMap::from([
        (
            "type".to_owned(),
            Json::Text(TRANSCRIPTION_COMPLETED.to_owned()),
        ),
        ("usage".to_owned(), usage.clone()),
    ]));
}

pub(crate) fn flush_unbilled_usage(state: &mut RealtimeStreaming) -> Result<(), SessionStateError> {
    let Some(config) = state.provider_config.as_deref() else {
        return Ok(());
    };
    let Some(usage) = config.unbilled_usage_on_close(&state.model) else {
        return Ok(());
    };
    let event = transcription_completed_record(usage)?;
    state.store_message(SessionMessage::Record(&event))?;
    state.capture_transcription_usage(&event);
    Ok(())
}

fn transcription_completed_record(
    usage: AudioTranscriptionUsage,
) -> Result<IndexMap<String, Json>, SessionStateError> {
    let encoded = serde_json::from_value(serde_json::to_value(usage)?)?;
    Ok(IndexMap::from([
        (
            "type".to_owned(),
            Json::Text(TRANSCRIPTION_COMPLETED.to_owned()),
        ),
        ("usage".to_owned(), encoded),
    ]))
}
