#[cfg(test)]
mod tests;
mod turns;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;

use self::turns::TurnState;
use self::turns::update_audio_progress;
use super::MuseMode;
use super::MuseProtocolError;
use super::PROVIDER_ERROR_MESSAGE;
use crate::api::transcription_protocol::JsonObject;
use crate::api::transcription_protocol::events::{
    CompletedEvent, DeltaEvent, ErrorEvent, EventIdSource, SpeechEvent,
    TranscriptionSessionCreated, error_event,
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

impl MuseEventTransformer {
    #[must_use]
    pub fn transform<Ids>(
        &mut self,
        frame: &JsonObject,
        mode: MuseMode,
        ids: &mut Ids,
    ) -> Result<Vec<MuseEvent>, MuseProtocolError>
    where
        Ids: EventIdSource + ?Sized,
    {
        match text_field(frame.get("type")) {
            Some("error") => Ok(vec![MuseEvent::Error(error_event(PROVIDER_ERROR_MESSAGE))]),
            Some("audioProgress") => {
                update_audio_progress(&mut self.last_ms, &mut self.unbilled_seconds, frame)?;
                Ok(Vec::new())
            }
            Some("speechStart") => self.speech_start(frame, ids),
            Some("transcript") => self.transcript(frame, mode, ids),
            Some("speechEnd") => self.speech_end(frame, ids),
            Some("speechComplete") => self.speech_complete(frame, ids),
            Some(_) | None => Ok(Vec::new()),
        }
    }
}
