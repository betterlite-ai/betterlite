mod json;
mod tests;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::Number;
use thiserror::Error;

use crate::api::api_boundaries::{ApiBoundariesInput, ApiLimitError, ApiLimits, EventBoundary};
use json::{field, present};

pub mod audio;
pub mod events;
pub mod usage;

pub type JsonObject = IndexMap<String, Json>;

const SESSION_UPDATE_EVENT_TYPES: [&str; 2] = ["session.update", "transcription_session.update"];
const SUPPORTED_TRANSCRIPTION_KEYS: [&str; 2] = ["model", "language"];

#[derive(Debug, Clone, PartialEq, Eq, Error)]
// ast-grep-ignore: no-long-names
pub enum RealtimeTranscriptionProtocolError {
    #[error("invalid JSON object")]
    InvalidJson,
    #[error("message must be a JSON object")]
    MessageNotObject,
    #[error("expected session.update")]
    ExpectedSessionUpdate,
    #[error("session.update requires a session object")]
    MissingSession,
    #[error("{name} must be an object")]
    FieldNotObject { name: &'static str },
    #[error("{name} must be a string")]
    FieldNotString { name: &'static str },
    #[error("{name} must be an integer")]
    FieldNotInteger { name: &'static str },
    #[error("input transcription must use either beta or GA layout")]
    TranscriptionLayoutConflict,
    #[error("input audio format must use either beta or GA layout")]
    AudioFormatLayoutConflict,
    #[error("Audio must be a base64 string")]
    AudioNotString,
    #[error("Audio append exceeds the four-second backlog limit")]
    AudioBacklog,
    #[error("Audio must be valid base64")]
    InvalidBase64,
    #[error("PCM16 audio must contain complete samples")]
    IncompleteSamples,
    #[error(transparent)]
    Limit(#[from] ApiLimitError),
}

// ast-grep-ignore: no-long-names
use self::RealtimeTranscriptionProtocolError as ProtocolError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TranscriptionLayout {
    Beta,
    Ga,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TranscriptionAudioFormat {
    pub layout: TranscriptionLayout,
    pub encoding: Option<String>,
    pub rate: Option<Number>,
    pub channels: Option<Number>,
}

impl TranscriptionAudioFormat {
    #[must_use]
    pub fn is_pcm16(&self) -> bool {
        self.encoding
            .as_deref()
            .is_some_and(|encoding| matches!(encoding, "pcm16" | "audio/pcm"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TranscriptionSessionUpdate {
    pub session_type: Option<String>,
    pub audio_format: Option<TranscriptionAudioFormat>,
    pub model: Option<String>,
    pub language: Option<String>,
    pub unsupported_transcription_keys: Vec<String>,
    pub turn_detection: Option<JsonObject>,
    pub turn_detection_disabled: bool,
}

impl TranscriptionSessionUpdate {
    #[must_use]
    pub fn turn_detection_type(&self) -> Option<&Json> {
        self.turn_detection
            .as_ref()
            .and_then(|fields| fields.get("type"))
            .filter(|value| !matches!(value, Json::Null))
    }
}

pub fn json_object<E>(payload: &[u8]) -> Result<JsonObject, E>
where
    E: From<ProtocolError>,
{
    match serde_json::from_slice::<Json>(payload) {
        Ok(Json::Object(fields)) => Ok(fields),
        Ok(_) => Err(E::from(ProtocolError::MessageNotObject)),
        Err(_) => Err(E::from(ProtocolError::InvalidJson)),
    }
}

struct SessionParts<'message> {
    session: &'message JsonObject,
    audio_input: Option<&'message JsonObject>,
    transcription: Option<&'message JsonObject>,
}

fn session_parts<E>(message: &JsonObject) -> Result<SessionParts<'_>, E>
where
    E: From<ProtocolError>,
{
    let Some(session) = field::<&JsonObject, E>(message.get("session"), "session")?
        .filter(|session| !session.is_empty())
    else {
        return Err(E::from(ProtocolError::MissingSession));
    };
    let audio = field::<&JsonObject, E>(session.get("audio"), "session.audio")?;
    let audio_input = field::<&JsonObject, E>(
        audio.and_then(|audio| audio.get("input")),
        "session.audio.input",
    )?;
    let beta = present(session.get("input_audio_transcription"));
    let ga = present(audio_input.and_then(|input| input.get("transcription")));
    if beta.is_some() && ga.is_some() {
        return Err(E::from(ProtocolError::TranscriptionLayoutConflict));
    }
    let transcription = field::<&JsonObject, E>(beta.or(ga), "input audio transcription")?;
    Ok(SessionParts {
        session,
        audio_input,
        transcription,
    })
}

fn session_turn_detection<E>(
    session: &JsonObject,
    audio_input: Option<&JsonObject>,
) -> Result<(Option<JsonObject>, bool), E>
where
    E: From<ProtocolError>,
{
    let given = session.contains_key("turn_detection")
        || audio_input.is_some_and(|input| input.contains_key("turn_detection"));
    let value = present(
        session
            .get("turn_detection")
            .or_else(|| audio_input.and_then(|input| input.get("turn_detection"))),
    );
    let detection = field::<&JsonObject, E>(value, "turn_detection")?;
    Ok((detection.cloned(), given && detection.is_none()))
}

// ast-grep-ignore: no-long-names
pub fn parse_transcription_session_update<E>(
    bytes: &[u8],
    limits: ApiLimits,
) -> Result<TranscriptionSessionUpdate, E>
where
    E: From<ProtocolError>,
{
    limits
        .check(ApiBoundariesInput::Event(EventBoundary {
            known_bytes: bytes.len(),
        }))
        .map_err(ProtocolError::from)?;
    let message = json_object::<E>(bytes)?;
    let wanted = message.get("type");
    if !matches!(wanted, Some(Json::Text(text)) if SESSION_UPDATE_EVENT_TYPES.contains(&text.as_str()))
    {
        return Err(E::from(ProtocolError::ExpectedSessionUpdate));
    }
    let parts = session_parts(&message)?;
    let session_type =
        field::<&str, E>(parts.session.get("type"), "session.type")?.map(str::to_owned);
    let audio_format = parse_audio_format(parts.session, parts.audio_input)?;
    let model = field::<&str, E>(
        parts.transcription.and_then(|fields| fields.get("model")),
        "transcription model",
    )?
    .map(str::to_owned);
    let language = field::<&str, E>(
        parts
            .transcription
            .and_then(|fields| fields.get("language")),
        "language",
    )?
    .map(str::to_owned);
    let (turn_detection, disabled) = session_turn_detection(parts.session, parts.audio_input)?;
    Ok(TranscriptionSessionUpdate {
        session_type,
        audio_format,
        model,
        language,
        unsupported_transcription_keys: unsupported_keys(parts.transcription),
        turn_detection,
        turn_detection_disabled: disabled,
    })
}

fn unsupported_keys(transcription: Option<&JsonObject>) -> Vec<String> {
    let Some(fields) = transcription else {
        return Vec::new();
    };
    let mut keys: Vec<String> = fields
        .keys()
        .filter(|key| !SUPPORTED_TRANSCRIPTION_KEYS.contains(&key.as_str()))
        .cloned()
        .collect();
    keys.sort();
    keys
}

fn ga_audio_format<E>(format: &Json) -> Result<TranscriptionAudioFormat, E>
where
    E: From<ProtocolError>,
{
    if let Json::Text(encoding) = format {
        return Ok(TranscriptionAudioFormat {
            layout: TranscriptionLayout::Ga,
            encoding: Some(encoding.clone()),
            rate: None,
            channels: None,
        });
    }
    let fields = match format {
        Json::Object(fields) => fields,
        _ => {
            return Err(E::from(ProtocolError::FieldNotObject {
                name: "session.audio.input.format",
            }));
        }
    };
    Ok(TranscriptionAudioFormat {
        layout: TranscriptionLayout::Ga,
        encoding: field::<&str, E>(fields.get("type"), "session.audio.input.format.type")?
            .map(str::to_owned),
        rate: field::<Number, E>(fields.get("rate"), "session.audio.input.format.rate")?,
        channels: field::<Number, E>(
            fields.get("channels"),
            "session.audio.input.format.channels",
        )?,
    })
}

fn parse_audio_format<E>(
    session: &JsonObject,
    audio_input: Option<&JsonObject>,
) -> Result<Option<TranscriptionAudioFormat>, E>
where
    E: From<ProtocolError>,
{
    let beta_format = present(session.get("input_audio_format"));
    let ga_format = present(audio_input.and_then(|input| input.get("format")));
    if beta_format.is_some() && ga_format.is_some() {
        return Err(E::from(ProtocolError::AudioFormatLayoutConflict));
    }
    if let Some(format) = beta_format {
        return Ok(Some(TranscriptionAudioFormat {
            layout: TranscriptionLayout::Beta,
            encoding: field::<&str, E>(Some(format), "session.input_audio_format")?
                .map(str::to_owned),
            rate: None,
            channels: None,
        }));
    }
    match ga_format {
        Some(format) => Ok(Some(ga_audio_format(format)?)),
        None => Ok(None),
    }
}
