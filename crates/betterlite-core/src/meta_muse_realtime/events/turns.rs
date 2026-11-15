use std::collections::BTreeSet;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use indexmap::map::Entry;
use serde_json::Number;
use uuid::Uuid;

use super::super::{MuseMode, MuseProtocolError, TURN_LIMIT};
use super::MuseEvent;
use super::MuseEventTransformer;
use crate::api::transcription_protocol::JsonObject;
use crate::api::transcription_protocol::events::{
    EventIdSource, SpeechEventType, completed_event, delta_event, speech_event,
};
use crate::api::transcription_protocol::usage::{TranscriptionUsage, duration_usage};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Emitted {
    SpeechStarted,
    SpeechStopped,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SpeechKind<'a> {
    Start,
    End,
    Complete { transcript: &'a str },
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

#[derive(Debug)]
pub(super) struct TurnState {
    item_id: String,
    started: bool,
    stopped: bool,
    latest_partial: Option<String>,
    emitted_partial: String,
    final_text: Option<String>,
    emitted: BTreeSet<Emitted>,
}

impl TurnState {
    fn new(item_id: &str) -> Self {
        Self {
            item_id: item_id.to_owned(),
            started: false,
            stopped: false,
            latest_partial: None,
            emitted_partial: String::new(),
            final_text: None,
            emitted: BTreeSet::new(),
        }
    }

    fn drain<Ids>(&mut self, ids: &mut Ids, unbilled: &mut f64) -> Vec<MuseEvent>
    where
        Ids: EventIdSource + ?Sized,
    {
        let mut events = Vec::new();
        let has_content = self.latest_partial.is_some() || self.final_text.is_some();
        if (self.started || has_content) && self.emitted.insert(Emitted::SpeechStarted) {
            self.push_speech(&mut events, ids, SpeechEventType::Started);
        }
        self.drain_partial(&mut events, ids);
        if self.stopped && self.emitted.insert(Emitted::SpeechStopped) {
            self.push_speech(&mut events, ids, SpeechEventType::Stopped);
        }
        if self.final_text.is_some()
            && self.emitted.contains(&Emitted::SpeechStopped)
            && self.emitted.insert(Emitted::Completed)
        {
            self.push_completed(&mut events, ids, unbilled);
        }
        events
    }

    fn drain_partial<Ids>(&mut self, events: &mut Vec<MuseEvent>, ids: &mut Ids)
    where
        Ids: EventIdSource + ?Sized,
    {
        if self.final_text.is_some() {
            return;
        }
        let Some(partial) = self.latest_partial.as_deref() else {
            return;
        };
        let suffix = partial
            .strip_prefix(self.emitted_partial.as_str())
            .unwrap_or_default();
        if suffix.is_empty() {
            return;
        }
        let event_id = ids.next_event_id();
        events.push(MuseEvent::Delta(delta_event(
            event_id,
            &self.item_id,
            suffix,
        )));
        self.emitted_partial = partial.to_owned();
    }

    fn push_speech<Ids>(&self, events: &mut Vec<MuseEvent>, ids: &mut Ids, kind: SpeechEventType)
    where
        Ids: EventIdSource + ?Sized,
    {
        let event_id = ids.next_event_id();
        events.push(MuseEvent::Speech(speech_event(
            event_id,
            kind,
            &self.item_id,
        )));
    }

    fn push_completed<Ids>(&self, events: &mut Vec<MuseEvent>, ids: &mut Ids, unbilled: &mut f64)
    where
        Ids: EventIdSource + ?Sized,
    {
        let usage = take_usage(unbilled);
        let transcript = self.final_text.as_deref().unwrap_or_default();
        let event_id = ids.next_event_id();
        let completed = completed_event(event_id, &self.item_id, transcript, usage);
        events.push(MuseEvent::Completed(completed));
    }
}

impl MuseEventTransformer {
    #[must_use]
    // ast-grep-ignore: no-one-line-functions
    pub fn take_unbilled_usage(&mut self) -> Option<TranscriptionUsage> {
        take_usage(&mut self.unbilled_seconds)
    }

    pub(super) fn speech<Ids>(
        &mut self,
        frame: &JsonObject,
        kind: SpeechKind<'_>,
        ids: &mut Ids,
    ) -> Result<Vec<MuseEvent>, MuseProtocolError>
    where
        Ids: EventIdSource + ?Sized,
    {
        let turn_id = required_turn_id(frame, kind.label())?;
        let state = turn(&mut self.turns, &turn_id);
        match kind {
            SpeechKind::Complete { transcript } => {
                finish(state, transcript);
                release_active(&mut self.active_turn_id, state);
            }
            SpeechKind::End => state.stopped = true,
            SpeechKind::Start if state.stopped => {}
            SpeechKind::Start => {
                state.started = true;
                self.active_turn_id = Some(state.item_id.clone());
            }
        }
        Ok(state.drain(ids, &mut self.unbilled_seconds))
    }

    pub(super) fn transcript<Ids>(
        &mut self,
        frame: &JsonObject,
        mode: MuseMode,
        ids: &mut Ids,
    ) -> Result<Vec<MuseEvent>, MuseProtocolError>
    where
        Ids: EventIdSource + ?Sized,
    {
        let Some(Json::Text(text)) = frame.get("transcript") else {
            return Err(MuseProtocolError::InvalidTranscript {
                event: "transcript",
            });
        };
        let turn_id_missing = matches!(frame.get("turnId"), None | Some(Json::Null));
        if text.is_empty() && turn_id_missing && self.active_turn_id.is_none() {
            return Ok(Vec::new());
        }
        let turn_id = transcript_turn_id(frame, &mut self.active_turn_id, mode)?;
        let state = turn(&mut self.turns, &turn_id);
        if matches!(frame.get("final"), Some(Json::Bool(true))) {
            finish(state, text);
            release_active(&mut self.active_turn_id, state);
        }
        if state.final_text.is_none() {
            state.latest_partial = Some(text.clone());
        }
        Ok(state.drain(ids, &mut self.unbilled_seconds))
    }
}

fn finish(turn: &mut TurnState, transcript: &str) {
    turn.final_text = Some(transcript.to_owned());
    turn.stopped = true;
}

fn release_active(active: &mut Option<String>, turn: &TurnState) {
    if active.as_deref() == Some(turn.item_id.as_str()) {
        *active = None;
    }
}

fn take_usage(unbilled: &mut f64) -> Option<TranscriptionUsage> {
    let seconds = *unbilled;
    if seconds <= 0.0 {
        return None;
    }
    *unbilled = 0.0;
    Some(duration_usage(seconds))
}

fn turn<'turns>(
    turns: &'turns mut IndexMap<String, TurnState>,
    turn_id: &str,
) -> &'turns mut TurnState {
    if !turns.contains_key(turn_id) && turns.len() >= TURN_LIMIT {
        drop(turns.shift_remove_index(0));
    }
    match turns.entry(turn_id.to_owned()) {
        Entry::Occupied(occupied) => occupied.into_mut(),
        Entry::Vacant(vacant) => vacant.insert(TurnState::new(turn_id)),
    }
}

fn required_turn_id(frame: &JsonObject, event: &'static str) -> Result<String, MuseProtocolError> {
    let text = match frame.get("turnId") {
        Some(Json::Text(text)) => Some(text.clone()),
        Some(Json::Number(number)) => integer_text(number),
        Some(Json::Null | Json::Bool(_) | Json::List(_) | Json::Object(_)) | None => None,
    };
    let Some(text) = text else {
        return Err(MuseProtocolError::InvalidTurnId { event });
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(MuseProtocolError::InvalidTurnId { event });
    }
    Ok(trimmed.to_owned())
}

fn integer_text(number: &Number) -> Option<String> {
    let text = number.to_string();
    // json keeps the -0 literal, while Python parses it to the integer 0 and prints "0"
    if text == "-0" {
        return Some("0".to_owned());
    }
    let unsigned = text.strip_prefix('-').unwrap_or(text.as_str());
    let integer = !unsigned.is_empty() && unsigned.bytes().all(|byte| byte.is_ascii_digit());
    integer.then_some(text)
}

fn transcript_turn_id(
    frame: &JsonObject,
    active: &mut Option<String>,
    mode: MuseMode,
) -> Result<String, MuseProtocolError> {
    if !matches!(frame.get("turnId"), None | Some(Json::Null)) {
        return required_turn_id(frame, "transcript");
    }
    if let Some(turn_id) = active.as_ref() {
        return Ok(turn_id.clone());
    }
    if mode != MuseMode::PushToTalk {
        return Err(MuseProtocolError::MissingTurnId);
    }
    let turn_id = format!("item_{}", Uuid::new_v4().simple());
    *active = Some(turn_id.clone());
    Ok(turn_id)
}
