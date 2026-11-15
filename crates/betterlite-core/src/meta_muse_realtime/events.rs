mod tests;
mod turns;

use std::ops::{Add, Div, Sub};

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;

use self::turns::TurnState;
use super::MILLIS_PER_SECOND;
use super::MuseProtocolError;
use crate::api::transcription_protocol::JsonObject;
use crate::api::transcription_protocol::events::{
    CompletedEvent, DeltaEvent, ErrorEvent, SpeechEvent, TranscriptionSessionCreated,
};

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum MuseEvent {
    Created(TranscriptionSessionCreated),
    Error(ErrorEvent),
    Speech(SpeechEvent),
    Delta(DeltaEvent),
    Completed(CompletedEvent),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SpeechKind<'text> {
    Start,
    End,
    Complete { transcript: &'text str },
}

impl SpeechKind<'_> {
    const fn label(self) -> &'static str {
        match self {
            Self::Start => "speechStart",
            Self::End => "speechEnd",
            Self::Complete { .. } => "speechComplete",
        }
    }
}

pub fn event_json(event: &MuseEvent) -> Result<Json, MuseProtocolError> {
    let text =
        serde_json::to_string(event).map_err(|source| MuseProtocolError::Encode { source })?;
    serde_json::from_str(&text).map_err(|source| MuseProtocolError::Encode { source })
}

pub(crate) fn text_field(value: Option<&Json>) -> Option<&str> {
    match value {
        Some(Json::Text(text)) => Some(text),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

#[derive(Debug, Default)]
pub struct MuseEventTransformer {
    turns: IndexMap<String, TurnState>,
    active_turn_id: Option<String>,
    last_ms: f64,
    unbilled_seconds: f64,
}

fn update_audio_progress(
    last_ms: &mut f64,
    unbilled: &mut f64,
    frame: &JsonObject,
) -> Result<(), MuseProtocolError> {
    let Some(Json::Number(number)) = frame.get("audioProcessedMs") else {
        return Err(MuseProtocolError::InvalidAudioProgress);
    };
    let Some(processed_ms) = number
        .as_f64()
        .filter(|value| value.is_finite() && *value >= 0.0_f64)
    else {
        return Err(MuseProtocolError::InvalidAudioProgress);
    };
    if processed_ms <= *last_ms {
        return Ok(());
    }
    let elapsed_ms = Sub::sub(processed_ms, *last_ms);
    *unbilled = Add::add(
        *unbilled,
        Div::div(elapsed_ms, f64::from(MILLIS_PER_SECOND)),
    );
    *last_ms = processed_ms;
    Ok(())
}
