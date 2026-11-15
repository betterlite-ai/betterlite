use serde::Serialize;
use uuid::Uuid;

use super::usage::TranscriptionUsage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ServerVadKind {
    #[serde(rename = "server_vad")]
    ServerVad,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ServerVadTurnDetection {
    #[serde(rename = "type")]
    pub kind: ServerVadKind,
}

pub const SERVER_VAD_TURN_DETECTION: ServerVadTurnDetection = ServerVadTurnDetection {
    kind: ServerVadKind::ServerVad,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum AudioFormatType {
    #[serde(rename = "audio/pcm")]
    AudioPcm,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionFormat {
    #[serde(rename = "type")]
    pub format_type: AudioFormatType,
    pub rate: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TranscriptionSettings {
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionAudioInput {
    pub format: SessionFormat,
    pub transcription: TranscriptionSettings,
    pub turn_detection: Option<ServerVadTurnDetection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SessionAudio {
    pub input: SessionAudioInput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SessionObject {
    #[serde(rename = "realtime.transcription_session")]
    RealtimeTranscriptionSession,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SessionType {
    #[serde(rename = "transcription")]
    Transcription,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TranscriptionSession {
    pub id: String,
    pub object: SessionObject,
    #[serde(rename = "type")]
    pub session_type: SessionType,
    pub audio: SessionAudio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SessionCreatedKind {
    #[serde(rename = "session.created")]
    SessionCreated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TranscriptionSessionCreated {
    #[serde(rename = "type")]
    pub kind: SessionCreatedKind,
    pub event_id: String,
    pub session: TranscriptionSession,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ErrorKind {
    #[serde(rename = "error")]
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ServerErrorKind {
    #[serde(rename = "server_error")]
    ServerError,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ErrorDetail {
    #[serde(rename = "type")]
    pub kind: ServerErrorKind,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ErrorEvent {
    #[serde(rename = "type")]
    pub kind: ErrorKind,
    pub error: ErrorDetail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SpeechEventType {
    #[serde(rename = "input_audio_buffer.speech_started")]
    Started,
    #[serde(rename = "input_audio_buffer.speech_stopped")]
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SpeechEvent {
    #[serde(rename = "type")]
    pub kind: SpeechEventType,
    pub event_id: String,
    pub item_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum DeltaKind {
    #[serde(rename = "conversation.item.input_audio_transcription.delta")]
    Delta,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeltaEvent {
    #[serde(rename = "type")]
    pub kind: DeltaKind,
    pub event_id: String,
    pub item_id: String,
    pub content_index: u8,
    pub delta: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CompletedKind {
    #[serde(rename = "conversation.item.input_audio_transcription.completed")]
    Completed,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CompletedEvent {
    #[serde(rename = "type")]
    pub kind: CompletedKind,
    pub event_id: String,
    pub item_id: String,
    pub content_index: u8,
    pub transcript: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TranscriptionUsage>,
}

pub trait EventIdSource {
    fn next_event_id(&mut self) -> String;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct UuidEventIds;

impl EventIdSource for UuidEventIds {
    fn next_event_id(&mut self) -> String {
        format!("event_{}", Uuid::new_v4().simple())
    }
}

#[must_use]
// ast-grep-ignore: no-long-names
pub fn transcription_session_created_event(
    event_id: String,
    session: &TranscriptionSession,
) -> TranscriptionSessionCreated {
    TranscriptionSessionCreated {
        kind: SessionCreatedKind::SessionCreated,
        event_id,
        session: session.clone(),
    }
}

#[must_use]
pub fn transcription_session(
    session_id: &str,
    model: &str,
    sample_rate: i64,
    language: Option<&str>,
    server_vad: bool,
) -> TranscriptionSession {
    TranscriptionSession {
        id: session_id.to_owned(),
        object: SessionObject::RealtimeTranscriptionSession,
        session_type: SessionType::Transcription,
        audio: SessionAudio {
            input: SessionAudioInput {
                format: SessionFormat {
                    format_type: AudioFormatType::AudioPcm,
                    rate: sample_rate,
                },
                transcription: TranscriptionSettings {
                    model: model.to_owned(),
                    language: language.map(str::to_owned),
                },
                turn_detection: server_vad.then_some(SERVER_VAD_TURN_DETECTION),
            },
        },
    }
}

#[must_use]
pub fn error_event(message: &str) -> ErrorEvent {
    ErrorEvent {
        kind: ErrorKind::Error,
        error: ErrorDetail {
            kind: ServerErrorKind::ServerError,
            message: message.to_owned(),
        },
    }
}

#[must_use]
pub fn speech_event(event_id: String, kind: SpeechEventType, item_id: &str) -> SpeechEvent {
    SpeechEvent {
        kind,
        event_id,
        item_id: item_id.to_owned(),
    }
}

#[must_use]
pub fn delta_event(event_id: String, item_id: &str, delta: &str) -> DeltaEvent {
    DeltaEvent {
        kind: DeltaKind::Delta,
        event_id,
        item_id: item_id.to_owned(),
        content_index: 0,
        delta: delta.to_owned(),
    }
}

#[must_use]
pub fn completed_event(
    event_id: String,
    item_id: &str,
    transcript: &str,
    usage: Option<TranscriptionUsage>,
) -> CompletedEvent {
    CompletedEvent {
        kind: CompletedKind::Completed,
        event_id,
        item_id: item_id.to_owned(),
        content_index: 0,
        transcript: transcript.to_owned(),
        usage,
    }
}
