use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::api::transcription_protocol::events::{
    CompletedEvent, DeltaEvent, SpeechEvent, TranscriptionSessionCreated,
};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChirpSpeechEvent {
    None,
    Begin,
    End,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ChirpStreamingResult {
    pub transcript: String,
    pub is_final: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ChirpStreamingResponse {
    pub speech_event: ChirpSpeechEvent,
    pub results: Vec<ChirpStreamingResult>,
    #[serde(with = "rust_decimal::serde::float")]
    pub billed_seconds: Decimal,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ChirpTurnDiscarded {
    #[serde(with = "rust_decimal::serde::float")]
    pub billed_seconds: Decimal,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChirpBackendEvent {
    Response(ChirpStreamingResponse),
    Configured,
    TurnFinished,
    TurnDiscarded(ChirpTurnDiscarded),
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ChirpEvent {
    SessionCreated(TranscriptionSessionCreated),
    Speech(SpeechEvent),
    Delta(DeltaEvent),
    Completed(CompletedEvent),
}
