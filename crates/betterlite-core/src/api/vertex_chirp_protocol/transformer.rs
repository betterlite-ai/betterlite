use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use uuid::Uuid;

use crate::api::transcription_protocol::events::{
    EventIdSource, SpeechEventType, completed_event, delta_event, speech_event,
};
use crate::api::transcription_protocol::usage::{TranscriptionUsage, duration_usage};

use super::events::{
    ChirpBackendEvent, ChirpEvent, ChirpSpeechEvent, ChirpStreamingResponse, ChirpStreamingResult,
};
use super::words::{join_transcript, new_words};
use super::{ChirpProtocolError, ChirpSessionConfig, session_created_event};

pub trait ItemIdSource {
    fn next_item_id(&mut self) -> String;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct UuidItemIds;

impl ItemIdSource for UuidItemIds {
    fn next_item_id(&mut self) -> String {
        format!("item_{}", Uuid::new_v4().simple())
    }
}

struct Configured {
    config: ChirpSessionConfig,
    session_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Turn {
    item_id: String,
    committed: String,
    preview: String,
    started_emitted: bool,
    stopped_emitted: bool,
}

pub struct ChirpEventTransformer<EventIds, ItemIds> {
    event_ids: EventIds,
    item_ids: ItemIds,
    configured: Option<Configured>,
    turn: Option<Turn>,
    billed_seconds: Decimal,
    reported_seconds: Decimal,
}

impl<EventIds, ItemIds> ChirpEventTransformer<EventIds, ItemIds>
where
    EventIds: EventIdSource,
    ItemIds: ItemIdSource,
{
    #[must_use]
    pub const fn new(event_ids: EventIds, item_ids: ItemIds) -> Self {
        Self {
            event_ids,
            item_ids,
            configured: None,
            turn: None,
            billed_seconds: Decimal::ZERO,
            reported_seconds: Decimal::ZERO,
        }
    }

    pub fn configure(&mut self, config: ChirpSessionConfig, session_id: &str) {
        self.configured = Some(Configured {
            config,
            session_id: session_id.to_owned(),
        });
    }

    pub fn take_unbilled_usage(
        &mut self,
    ) -> Result<Option<TranscriptionUsage>, ChirpProtocolError> {
        let Some(unreported) = self.billed_seconds.checked_sub(self.reported_seconds) else {
            return Err(ChirpProtocolError::Billing);
        };
        if unreported <= Decimal::ZERO {
            return Ok(None);
        }
        self.reported_seconds = self.billed_seconds;
        let Some(seconds) = unreported.to_f64() else {
            return Err(ChirpProtocolError::Billing);
        };
        Ok(Some(duration_usage(seconds)))
    }

    pub fn transform(
        &mut self,
        event: ChirpBackendEvent,
    ) -> Result<Vec<ChirpEvent>, ChirpProtocolError> {
        match event {
            ChirpBackendEvent::Configured => Ok(vec![self.session_created()?]),
            ChirpBackendEvent::Response(response) => self.response(&response),
            ChirpBackendEvent::TurnFinished => self.finish_turn(),
            ChirpBackendEvent::TurnDiscarded(discarded) => {
                self.billed_seconds = self.billed_seconds.max(discarded.billed_seconds);
                self.turn = None;
                Ok(Vec::new())
            }
        }
    }

    fn session_created(&mut self) -> Result<ChirpEvent, ChirpProtocolError> {
        let configured = self
            .configured
            .as_ref()
            .ok_or(ChirpProtocolError::Unconfigured)?;
        let event_id = self.event_ids.next_event_id();
        Ok(ChirpEvent::SessionCreated(session_created_event(
            event_id,
            &configured.config,
            &configured.session_id,
        )))
    }

    fn response(
        &mut self,
        frame: &ChirpStreamingResponse,
    ) -> Result<Vec<ChirpEvent>, ChirpProtocolError> {
        self.billed_seconds = self.billed_seconds.max(frame.billed_seconds);
        let interim = interim_text(&frame.results);
        let finals = final_texts(&frame.results);
        let mut events = Vec::new();
        if frame.speech_event == ChirpSpeechEvent::Begin {
            events.extend(self.begin()?);
        }
        for text in finals {
            events.extend(self.final_result(text)?);
        }
        if !interim.is_empty() {
            events.extend(self.hypothesis(&interim)?);
        }
        if frame.speech_event == ChirpSpeechEvent::End {
            events.extend(self.stop()?);
        }
        Ok(events)
    }

    fn begin(&mut self) -> Result<Vec<ChirpEvent>, ChirpProtocolError> {
        let server_vad = self
            .configured
            .as_ref()
            .ok_or(ChirpProtocolError::Unconfigured)?
            .config
            .server_vad;
        let turn = self.require_turn();
        if turn.started_emitted || !server_vad {
            return Ok(Vec::new());
        }
        turn.started_emitted = true;
        let item_id = turn.item_id.clone();
        Ok(vec![ChirpEvent::Speech(speech_event(
            self.event_ids.next_event_id(),
            SpeechEventType::Started,
            &item_id,
        ))])
    }

    fn stop(&mut self) -> Result<Vec<ChirpEvent>, ChirpProtocolError> {
        let Some(turn) = self.turn.as_mut() else {
            return Ok(Vec::new());
        };
        let server_vad = self
            .configured
            .as_ref()
            .ok_or(ChirpProtocolError::Unconfigured)?
            .config
            .server_vad;
        if turn.stopped_emitted || !server_vad {
            return Ok(Vec::new());
        }
        turn.stopped_emitted = true;
        let item_id = turn.item_id.clone();
        Ok(vec![ChirpEvent::Speech(speech_event(
            self.event_ids.next_event_id(),
            SpeechEventType::Stopped,
            &item_id,
        ))])
    }

    fn hypothesis(&mut self, text: &str) -> Result<Vec<ChirpEvent>, ChirpProtocolError> {
        let mut events = self.begin()?;
        let (item_id, delta) = {
            let turn = self.require_turn();
            let joined = join_transcript(&turn.committed, text);
            let delta = new_words(&turn.preview, &joined);
            turn.preview = joined;
            (turn.item_id.clone(), delta)
        };
        if !delta.is_empty() {
            events.push(ChirpEvent::Delta(delta_event(
                self.event_ids.next_event_id(),
                &item_id,
                &delta,
            )));
        }
        Ok(events)
    }

    fn final_result(&mut self, text: &str) -> Result<Vec<ChirpEvent>, ChirpProtocolError> {
        let mut events = self.begin()?;
        let server_vad = self
            .configured
            .as_ref()
            .ok_or(ChirpProtocolError::Unconfigured)?
            .config
            .server_vad;
        let (item_id, delta) = {
            let turn = self.require_turn();
            let committed = join_transcript(&turn.committed, text);
            let delta = new_words(&turn.preview, &committed);
            turn.committed.clone_from(&committed);
            turn.preview = committed;
            (turn.item_id.clone(), delta)
        };
        if !delta.is_empty() {
            events.push(ChirpEvent::Delta(delta_event(
                self.event_ids.next_event_id(),
                &item_id,
                &delta,
            )));
        }
        if !server_vad {
            return Ok(events);
        }
        events.extend(self.complete()?);
        Ok(events)
    }

    fn complete(&mut self) -> Result<Vec<ChirpEvent>, ChirpProtocolError> {
        let (item_id, transcript) = {
            let turn = self.require_turn();
            let transcript = if turn.committed.is_empty() {
                turn.preview.clone()
            } else {
                turn.committed.clone()
            };
            (turn.item_id.clone(), transcript)
        };
        let mut events = self.stop()?;
        self.turn = None;
        let usage = self.take_unbilled_usage()?;
        events.push(ChirpEvent::Completed(completed_event(
            self.event_ids.next_event_id(),
            &item_id,
            &transcript,
            usage,
        )));
        Ok(events)
    }

    fn finish_turn(&mut self) -> Result<Vec<ChirpEvent>, ChirpProtocolError> {
        if self.turn.is_none() {
            return Ok(Vec::new());
        }
        self.complete()
    }

    fn require_turn(&mut self) -> &mut Turn {
        let item_ids = &mut self.item_ids;
        self.turn.get_or_insert_with(|| Turn {
            item_id: item_ids.next_item_id(),
            committed: String::new(),
            preview: String::new(),
            started_emitted: false,
            stopped_emitted: false,
        })
    }
}

fn interim_text(results: &[ChirpStreamingResult]) -> String {
    results
        .iter()
        .filter(|result| !result.is_final)
        .map(|result| result.transcript.trim())
        .filter(|transcript| !transcript.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn final_texts(results: &[ChirpStreamingResult]) -> Vec<&str> {
    results
        .iter()
        .filter(|result| result.is_final)
        .map(|result| result.transcript.trim())
        .filter(|transcript| !transcript.is_empty())
        .collect()
}
