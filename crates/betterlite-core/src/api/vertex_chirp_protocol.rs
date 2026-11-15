mod command;
pub mod events;
mod tests;
pub mod transformer;
pub mod words;

use betterlite_models::catalog::ModelCatalog;
use betterlite_models::decisions::Json;
use betterlite_models::lookup::info::ModelQuery;
use betterlite_models::lookup::resolve::find_info;
use betterlite_models::metadata::identity::Mode;
use betterlite_openai::api::audio_file_input::language::normalize_language_to_bcp47;
use betterlite_tokenizer::model_capabilities::BEDROCK_NAMES;
use command::{CommandKind, ConfigureCommand};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::api::api_boundaries::ApiLimits;
use crate::api::transcription_protocol::events::{
    TranscriptionSession,
    transcription_session,
    // ast-grep-ignore: no-long-names
    transcription_session_created_event as created_event,
};
use crate::api::transcription_protocol::{
    // ast-grep-ignore: no-long-names
    RealtimeTranscriptionProtocolError as ProtocolError,
    TranscriptionAudioFormat,
    TranscriptionSessionUpdate,
    // ast-grep-ignore: no-long-names
    parse_transcription_session_update as parse_update,
};

pub const DEFAULT_SAMPLE_RATE_HERTZ: i64 = 24_000;
pub const MIN_SAMPLE_RATE_HERTZ: i64 = 8_000;
pub const MAX_SAMPLE_RATE_HERTZ: i64 = 48_000;
pub const MAX_AUDIO_MESSAGE_BYTES: usize = 25_000;
pub const AUTO_LANGUAGE_CODE: &str = "auto";
pub const FINISH_TURN_COMMAND: &str = r#"{"kind":"finish_turn"}"#;
pub const DISCARD_TURN_COMMAND: &str = r#"{"kind":"discard_turn"}"#;

const SPEECH_TO_TEXT_ENDPOINTS: [&str; 2] = ["/v1/audio/transcriptions", "/v1/realtime"];
const VERTEX_MODEL_PREFIX: &str = "vertex_ai/";
const PROVIDER: &str = "vertex_ai";

#[derive(Debug, Error)]
pub enum ChirpProtocolError {
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    #[error("Speech-to-Text streaming supports transcription sessions only")]
    SessionType,
    #[error("realtime session model cannot be changed")]
    ModelChanged,
    #[error("Speech-to-Text streaming requires pcm16 input audio")]
    NotPcm16,
    #[error("Speech-to-Text streaming requires mono input audio")]
    NotMono,
    #[error("Speech-to-Text streaming supports sample rates from {min} Hz to {max} Hz")]
    SampleRate { min: i64, max: i64 },
    #[error("Speech-to-Text streaming supports server_vad turn detection or null")]
    TurnDetection,
    #[error("session.update must configure the session before the backend responds")]
    Unconfigured,
    #[error("the configure command cannot be serialized")]
    Command(#[source] serde_json::Error),
    #[error("billed seconds cannot be represented as a float")]
    Billing,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct ChirpSessionConfig {
    pub model: String,
    pub language: Option<String>,
    pub sample_rate: i64,
    pub server_vad: bool,
}

impl ChirpSessionConfig {
    #[must_use]
    pub fn openai_session(&self, session_id: &str) -> TranscriptionSession {
        transcription_session(
            session_id,
            &self.model,
            self.sample_rate,
            self.language.as_deref(),
            self.server_vad,
        )
    }

    pub fn configure_command(&self) -> Result<String, ChirpProtocolError> {
        let command = ConfigureCommand {
            kind: CommandKind::Configure,
            model: &self.model,
            language_codes: [self.language.as_deref().unwrap_or(AUTO_LANGUAGE_CODE)],
            sample_rate_hertz: self.sample_rate,
        };
        serde_json::to_string(&command).map_err(ChirpProtocolError::Command)
    }
}

#[must_use]
pub fn is_vertex_speech_to_text_model(catalog: &ModelCatalog, model: &str) -> bool {
    let normalized = normalize_speech_to_text_model(model);
    let query = ModelQuery {
        model: &normalized,
        provider: Some(PROVIDER),
    };
    let Ok(Some(resolved)) = find_info(catalog, &BEDROCK_NAMES, &query) else {
        return false;
    };
    resolved.mode == Some(Mode::AudioTranscription)
        && supports_speech_endpoints(catalog, &resolved.key)
}

fn supports_speech_endpoints(catalog: &ModelCatalog, key: &str) -> bool {
    catalog.entries.get(key).is_some_and(|entry| {
        SPEECH_TO_TEXT_ENDPOINTS.iter().all(|endpoint| {
            entry
                .fields
                .metadata
                .supported_endpoints
                .iter()
                .any(|supported| supported == endpoint)
        })
    })
}

#[must_use]
pub fn normalize_speech_to_text_model(model: &str) -> String {
    model
        .strip_prefix(VERTEX_MODEL_PREFIX)
        .map_or_else(|| model.to_owned(), str::to_owned)
}

#[must_use]
pub fn default_session_config(model: &str) -> ChirpSessionConfig {
    ChirpSessionConfig {
        model: normalize_speech_to_text_model(model),
        language: None,
        sample_rate: DEFAULT_SAMPLE_RATE_HERTZ,
        server_vad: true,
    }
}

pub fn parse_chirp_session_update(
    payload: &[u8],
    expected_model: &str,
    limits: ApiLimits,
) -> Result<ChirpSessionConfig, ChirpProtocolError> {
    let update = parse_update::<ChirpProtocolError>(payload, limits)?;
    if let Some(session_type) = update.session_type.as_deref()
        && !matches!(session_type, "transcription" | "realtime")
    {
        return Err(ChirpProtocolError::SessionType);
    }
    if !update.unsupported_transcription_keys.is_empty() {
        tracing::debug!(
            "Speech-to-Text streaming: ignoring unsupported transcription settings {:?}",
            update.unsupported_transcription_keys
        );
    }
    let model = normalize_speech_to_text_model(expected_model);
    if let Some(changed) = update.model.as_deref()
        && normalize_speech_to_text_model(changed) != model
    {
        return Err(ChirpProtocolError::ModelChanged);
    }
    Ok(ChirpSessionConfig {
        model,
        language: update.language.as_deref().map(normalize_language_to_bcp47),
        sample_rate: sample_rate_hertz(update.audio_format.as_ref())?,
        server_vad: server_vad(&update)?,
    })
}

fn sample_rate_hertz(audio: Option<&TranscriptionAudioFormat>) -> Result<i64, ChirpProtocolError> {
    let Some(format) = audio else {
        return Ok(DEFAULT_SAMPLE_RATE_HERTZ);
    };
    if !format.is_pcm16() {
        return Err(ChirpProtocolError::NotPcm16);
    }
    if format
        .channels
        .as_ref()
        .is_some_and(|channels| channels.as_i64() != Some(1))
    {
        return Err(ChirpProtocolError::NotMono);
    }
    let Some(rate) = format.rate.as_ref() else {
        return Ok(DEFAULT_SAMPLE_RATE_HERTZ);
    };
    let Some(hertz) = rate.as_i64() else {
        return Err(sample_rate_error());
    };
    if !(MIN_SAMPLE_RATE_HERTZ..=MAX_SAMPLE_RATE_HERTZ).contains(&hertz) {
        return Err(sample_rate_error());
    }
    Ok(hertz)
}

const fn sample_rate_error() -> ChirpProtocolError {
    ChirpProtocolError::SampleRate {
        min: MIN_SAMPLE_RATE_HERTZ,
        max: MAX_SAMPLE_RATE_HERTZ,
    }
}

fn server_vad(update: &TranscriptionSessionUpdate) -> Result<bool, ChirpProtocolError> {
    if update.turn_detection_disabled {
        return Ok(false);
    }
    match update.turn_detection_type() {
        None => Ok(true),
        Some(Json::Text(kind)) if kind == "server_vad" => Ok(true),
        Some(_) => Err(ChirpProtocolError::TurnDetection),
    }
}
