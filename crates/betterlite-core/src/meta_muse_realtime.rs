pub mod events;
pub mod request;
pub mod session;
mod support;

use secrecy::ExposeSecret as _;
use secrecy::SecretString;
use serde::Serialize;
use thiserror::Error;

// ast-grep-ignore: no-long-names
use crate::api::transcription_protocol::RealtimeTranscriptionProtocolError as ProtocolError;
use crate::api::transcription_protocol::events::{TranscriptionSession, transcription_session};

pub const MUSE_MODEL: &str = "muse-voice-transcribe-1.0";
pub const DEFAULT_MUSE_REALTIME_URL: &str = "wss://api.meta.ai/v1/asr/realtime";
pub const AUDIO_BACKLOG_SECONDS: u32 = 4;
pub const PACKET_MS: u32 = 80;
pub const MILLIS_PER_SECOND: u32 = 1_000;
pub const END_STREAM: &str = r#"{"type":"endStream"}"#;
pub const PROVIDER_ERROR_MESSAGE: &str = "Meta Muse realtime transcription failed";
pub const TURN_LIMIT: usize = 128;

pub const SUPPORTED_LANGUAGES: [&str; 25] = [
    "Arabic",
    "Bengali",
    "Dutch",
    "English",
    "French",
    "German",
    "Hebrew",
    "Hindi",
    "Indonesian",
    "Italian",
    "Japanese",
    "Kannada",
    "Korean",
    "Malay",
    "Mandarin Chinese",
    "Marathi",
    "Polish",
    "Portuguese",
    "Spanish",
    "Tagalog",
    "Tamil",
    "Telugu",
    "Thai",
    "Turkish",
    "Vietnamese",
];

pub const LANGUAGE_CODES: [(&str, &str); 27] = [
    ("ar", "Arabic"),
    ("bn", "Bengali"),
    ("de", "German"),
    ("en", "English"),
    ("es", "Spanish"),
    ("fil", "Tagalog"),
    ("fr", "French"),
    ("he", "Hebrew"),
    ("hi", "Hindi"),
    ("id", "Indonesian"),
    ("it", "Italian"),
    ("iw", "Hebrew"),
    ("ja", "Japanese"),
    ("kn", "Kannada"),
    ("ko", "Korean"),
    ("ms", "Malay"),
    ("mr", "Marathi"),
    ("nl", "Dutch"),
    ("pl", "Polish"),
    ("pt", "Portuguese"),
    ("ta", "Tamil"),
    ("te", "Telugu"),
    ("th", "Thai"),
    ("tl", "Tagalog"),
    ("tr", "Turkish"),
    ("vi", "Vietnamese"),
    ("zh", "Mandarin Chinese"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MuseMode {
    #[serde(rename = "PUSH_TO_TALK")]
    PushToTalk,
    #[serde(rename = "ENDPOINTING")]
    Endpointing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MuseSampleRate {
    Hz16000,
    Hz24000,
}

impl MuseSampleRate {
    #[must_use]
    pub const fn hertz(self) -> u32 {
        match self {
            Self::Hz16000 => 16_000,
            Self::Hz24000 => 24_000,
        }
    }

    #[must_use]
    pub const fn bytes_per_second(self) -> u32 {
        match self {
            Self::Hz16000 => 32_000,
            Self::Hz24000 => 48_000,
        }
    }

    #[must_use]
    pub fn packet_bytes(self) -> usize {
        usize::try_from(
            self.bytes_per_second()
                .saturating_mul(PACKET_MS)
                .saturating_div(MILLIS_PER_SECOND),
        )
        .unwrap_or(usize::MAX)
    }

    #[must_use]
    pub fn max_encoded_append_bytes(self) -> usize {
        let backlog = self
            .bytes_per_second()
            .saturating_mul(AUDIO_BACKLOG_SECONDS)
            .saturating_add(2);
        let groups = backlog.saturating_div(3);
        usize::try_from(groups.saturating_mul(4)).unwrap_or(usize::MAX)
    }

    #[must_use]
    pub const fn audio_encoding(self) -> MuseAudioEncoding {
        match self {
            Self::Hz16000 => MuseAudioEncoding::Pcm16Khz,
            Self::Hz24000 => MuseAudioEncoding::Pcm24Khz,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum MuseAudioEncoding {
    #[serde(rename = "PCM_16KHZ")]
    Pcm16Khz,
    #[serde(rename = "PCM_24KHZ")]
    Pcm24Khz,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum PartialMode {
    #[serde(rename = "CUMULATIVE")]
    Cumulative,
}

#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct MuseAuthorization {
    #[serde(rename = "accessToken")]
    access_token: String,
}

#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct MuseHandshake {
    pub authorization: MuseAuthorization,
    #[serde(rename = "audioEncoding")]
    pub audio_encoding: MuseAudioEncoding,
    pub model: String,
    pub mode: MuseMode,
    #[serde(rename = "partialMode")]
    pub partial_mode: PartialMode,
    #[serde(rename = "emitAudioProgress")]
    pub emit_audio_progress: bool,
    #[serde(rename = "languageBias", skip_serializing_if = "Option::is_none")]
    pub language_bias: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MuseSessionConfig {
    pub model: String,
    pub mode: MuseMode,
    pub sample_rate: MuseSampleRate,
    pub language_bias: Vec<String>,
}

impl MuseSessionConfig {
    #[must_use]
    pub fn handshake(&self, access_token: &SecretString) -> MuseHandshake {
        let language_bias = (!self.language_bias.is_empty()).then(|| self.language_bias.clone());
        MuseHandshake {
            authorization: MuseAuthorization {
                access_token: access_token.expose_secret().to_owned(),
            },
            audio_encoding: self.sample_rate.audio_encoding(),
            model: self.model.clone(),
            mode: self.mode,
            partial_mode: PartialMode::Cumulative,
            emit_audio_progress: true,
            language_bias,
        }
    }

    #[must_use]
    pub fn openai_session(&self, session_id: &str) -> TranscriptionSession {
        transcription_session(
            session_id,
            &self.model,
            i64::from(self.sample_rate.hertz()),
            self.language_bias.first().map(String::as_str),
            self.mode != MuseMode::PushToTalk,
        )
    }
}

impl Default for MuseSessionConfig {
    fn default() -> Self {
        Self {
            model: MUSE_MODEL.to_owned(),
            mode: MuseMode::Endpointing,
            sample_rate: MuseSampleRate::Hz24000,
            language_bias: Vec::new(),
        }
    }
}

#[derive(Debug, Error)]
pub enum MuseProtocolError {
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    #[error("language must be non-empty")]
    LanguageEmpty,
    #[error("unsupported Muse Voice language")]
    UnsupportedLanguage,
    #[error("Meta API key is required")]
    ApiKeyRequired,
    #[error("Meta API key must include a token after Bearer")]
    BearerTokenMissing,
    #[error(
        "Meta api_base must be an absolute wss:// or https:// URL without credentials or a fragment"
    )]
    InvalidApiBase,
    #[error("Muse Voice supports transcription sessions only")]
    TranscriptionOnly,
    #[error("unsupported Meta realtime model")]
    UnsupportedModel,
    #[error("realtime session model cannot be changed")]
    ModelChanged,
    #[error("Muse Voice requires pcm16 input audio")]
    BetaEncoding,
    #[error("Muse Voice requires audio/pcm input audio")]
    PcmEncoding,
    #[error("Muse Voice requires mono input audio")]
    MonoOnly,
    #[error("Muse Voice supports PCM16 at 16000 Hz or 24000 Hz")]
    SampleRateUnsupported,
    #[error("Muse Voice supports server_vad turn detection or null")]
    TurnDetectionUnsupported,
    #[error("{event} event has invalid turnId")]
    InvalidTurnId { event: &'static str },
    #[error("{event} event has invalid transcript")]
    InvalidTranscript { event: &'static str },
    #[error("audioProgress event has invalid audioProcessedMs")]
    InvalidAudioProgress,
    #[error("transcript event is missing turnId outside an active turn")]
    MissingTurnId,
    #[error("api_key is required for Meta API calls")]
    ApiKeyMissing,
    #[error("Unsupported Meta realtime model: {model}")]
    ModelRejected { model: String },
    #[error("session.update must configure the Muse session before audio is sent")]
    SessionNotConfigured,
    #[error("Meta API key was not validated before the session was configured")]
    TokenNotValidated,
    #[error("provider returned an invalid handshake response")]
    InvalidHandshake,
    #[error("the Muse message is too large to process")]
    MessageTooLarge,
    #[error("the Muse audio packet size is invalid")]
    InvalidPacketSize,
    #[error("the provider frame is not UTF-8")]
    FrameNotUtf8,
    #[error("the Muse frame could not be encoded")]
    Encode {
        #[source]
        source: serde_json::Error,
    },
}
