use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::transcription_protocol::audio::decode_pcm16_append;
use betterlite_core::api::transcription_protocol::events::{
    EventIdSource,
    SpeechEventType,
    TranscriptionSession,
    UuidEventIds,
    completed_event,
    delta_event,
    error_event,
    speech_event,
    transcription_session,
    // ast-grep-ignore: no-long-names
    transcription_session_created_event as created_event,
};
use betterlite_core::api::transcription_protocol::usage::{
    InputTokenDetails, TokenUsage, TranscriptionUsage, duration_usage,
};
use betterlite_core::api::transcription_protocol::{
    // ast-grep-ignore: no-long-names
    RealtimeTranscriptionProtocolError as ProtocolError,
    TranscriptionAudioFormat,
    TranscriptionSessionUpdate,
    // ast-grep-ignore: no-long-names
    parse_transcription_session_update as parse_update,
};
use betterlite_models::decisions::Json;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperResult};

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Function {
    Parse(ParseInput),
    Decode(DecodeInput),
    Event(Box<EventInput>),
}

#[derive(Debug, Deserialize)]
pub(crate) struct ParseInput {
    payloads: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DecodeInput {
    audio: Vec<Json>,
    max_encoded_bytes: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct TokenUsageDocument {
    #[serde(rename = "input_tokens")]
    input: i64,
    #[serde(rename = "output_tokens")]
    output: i64,
    #[serde(rename = "total_tokens")]
    total: i64,
    #[serde(rename = "text_tokens")]
    text: i64,
    #[serde(rename = "audio_tokens")]
    audio: i64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct EventInput {
    event_ids: Vec<String>,
    session_id: String,
    model: String,
    sample_rate: i64,
    language: Option<String>,
    server_vad: bool,
    item_id: String,
    delta: String,
    transcript: String,
    duration_seconds: f64,
    token_usage: TokenUsageDocument,
    error_message: String,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Function;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Function::Parse(fields) => Ok(parse(fields)),
            Function::Decode(fields) => Ok(decode(fields)),
            Function::Event(fields) => Ok(event(fields)),
        }
    }
}

fn parse(fields: &ParseInput) -> Value {
    let limits = ApiLimits::default();
    let sessions: Vec<Value> = fields
        .payloads
        .iter()
        .map(|payload| {
            parse_update::<ProtocolError>(payload.as_bytes(), limits)
                .map_or(Value::Null, |update| session_document(&update))
        })
        .collect();
    json!({ "sessions": sessions })
}

fn session_document(update: &TranscriptionSessionUpdate) -> Value {
    json!({
        "session_type": update.session_type,
        "audio_format": update.audio_format,
        "is_pcm16": update.audio_format.as_ref().map(TranscriptionAudioFormat::is_pcm16),
        "model": update.model,
        "language": update.language,
        "unsupported_transcription_keys": update.unsupported_transcription_keys,
        "turn_detection": update.turn_detection,
        "turn_detection_disabled": update.turn_detection_disabled,
        "turn_detection_type": update.turn_detection_type(),
    })
}

fn decode(fields: &DecodeInput) -> Value {
    let decoded: Vec<Value> = fields
        .audio
        .iter()
        .map(|audio| {
            decode_pcm16_append::<ProtocolError>(Some(audio), fields.max_encoded_bytes)
                .map_or(Value::Null, |bytes| json!(bytes))
        })
        .collect();
    json!({ "decoded": decoded })
}

fn event(fields: &EventInput) -> Value {
    let mut ids = FixedEventIds {
        ids: &fields.event_ids,
        next: 0,
    };
    let (session, without_language) = sessions(fields);
    json!({
        "generated_id_matches": generated_id_matches(),
        "created": created_event(ids.next_event_id(), &session),
        "speech_started": speech_event(ids.next_event_id(), SpeechEventType::Started, &fields.item_id),
        "speech_stopped": speech_event(ids.next_event_id(), SpeechEventType::Stopped, &fields.item_id),
        "delta": delta_event(ids.next_event_id(), &fields.item_id, &fields.delta),
        "completed_none": completed_event(ids.next_event_id(), &fields.item_id, &fields.transcript, None),
        "completed_duration": completed_event(ids.next_event_id(), &fields.item_id, &fields.transcript, Some(duration_usage(fields.duration_seconds))),
        "completed_tokens": completed_event(ids.next_event_id(), &fields.item_id, &fields.transcript, Some(token_usage(&fields.token_usage))),
        "error": error_event(&fields.error_message),
        "session": session,
        "session_without_language": without_language,
    })
}

fn sessions(fields: &EventInput) -> (TranscriptionSession, TranscriptionSession) {
    let session = transcription_session(
        &fields.session_id,
        &fields.model,
        fields.sample_rate,
        fields.language.as_deref(),
        fields.server_vad,
    );
    let without_language = transcription_session(
        &fields.session_id,
        &fields.model,
        fields.sample_rate,
        None,
        false,
    );
    (session, without_language)
}

fn token_usage(document: &TokenUsageDocument) -> TranscriptionUsage {
    TranscriptionUsage::Tokens(Box::new(TokenUsage {
        input_tokens: document.input,
        output_tokens: document.output,
        total_tokens: document.total,
        input_token_details: InputTokenDetails {
            text_tokens: document.text,
            audio_tokens: document.audio,
        },
    }))
}

fn generated_id_matches() -> bool {
    let mut ids = UuidEventIds;
    let identifier = ids.next_event_id();
    identifier.strip_prefix("event_").is_some_and(|hex| {
        hex.len() == 32
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

struct FixedEventIds<'ids> {
    ids: &'ids [String],
    next: usize,
}

impl EventIdSource for FixedEventIds<'_> {
    fn next_event_id(&mut self) -> String {
        let identifier = self.ids.get(self.next).cloned().unwrap_or_default();
        self.next = self.next.saturating_add(1);
        identifier
    }
}
