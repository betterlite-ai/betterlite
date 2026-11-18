use std::error::Error;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::error::RealtimeSenderError;

mod python_json;

type Map = IndexMap<String, Json>;

// LiteLLM's json.loads reads deeper frames than serde_json, so a frame past this depth is refused before any parse
const MAX_NESTING: usize = 64;

#[derive(Debug, Deserialize)]
struct Envelope {
    #[serde(rename = "type", default = "default_slot")]
    kind: Slot<UpdateType>,
    #[serde(default = "default_slot")]
    session: Slot<Map>,
}

#[derive(Debug, Deserialize, Serialize)]
enum UpdateType {
    #[serde(rename = "session.update")]
    Session,
    #[serde(rename = "transcription_session.update")]
    Transcription,
}

#[derive(Debug, Deserialize, Serialize)]
enum SessionType {
    #[serde(rename = "transcription")]
    Transcription,
}

// A session field is an object of the typed shape, any other JSON value it carries, or absent from the frame
#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
enum Slot<T> {
    // Recognize arbitrary-precision numbers before their internal map can match a typed object.
    Number(serde_json::Number),
    Object(T),
    Other(NonObject),
    Missing,
}

// No object alternative: a failed typed object (including duplicate fields) cannot become passthrough.
#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
enum NonObject {
    Null,
    Bool(bool),
    Text(String),
    List(Vec<Json>),
}

impl<T> Slot<T> {
    const fn is_missing(&self) -> bool {
        match self {
            Self::Missing => true,
            Self::Number(_) | Self::Object(_) | Self::Other(_) => false,
        }
    }
}

const fn default_slot<T>() -> Slot<T> {
    Slot::Missing
}

#[derive(Debug, Deserialize, Serialize)]
struct Update {
    #[serde(rename = "type")]
    kind: UpdateType,
    session: Session,
    #[serde(flatten)]
    rest: Map,
}

#[derive(Debug, Deserialize, Serialize)]
struct Session {
    #[serde(default = "default_slot", skip_serializing_if = "Slot::is_missing")]
    r#type: Slot<SessionType>,
    #[serde(default = "default_slot", skip_serializing_if = "Slot::is_missing")]
    input_audio_transcription: Slot<Transcription>,
    #[serde(default = "default_slot", skip_serializing_if = "Slot::is_missing")]
    audio: Slot<Audio>,
    #[serde(flatten)]
    rest: Map,
}

#[derive(Debug, Deserialize, Serialize)]
struct Audio {
    #[serde(default = "default_slot", skip_serializing_if = "Slot::is_missing")]
    input: Slot<AudioInput>,
    #[serde(flatten)]
    rest: Map,
}

#[derive(Debug, Deserialize, Serialize)]
struct AudioInput {
    #[serde(default = "default_slot", skip_serializing_if = "Slot::is_missing")]
    transcription: Slot<Transcription>,
    #[serde(flatten)]
    rest: Map,
}

#[derive(Debug, Deserialize, Serialize)]
struct Transcription {
    #[serde(default = "default_slot", skip_serializing_if = "Slot::is_missing")]
    model: Slot<String>,
    #[serde(flatten)]
    rest: Map,
}

impl Transcription {
    fn set_model(&mut self, authorized_model: &str) -> bool {
        if matches!(&self.model, Slot::Object(model) if model == authorized_model) {
            return false;
        }
        self.model = Slot::Object(authorized_model.to_owned());
        true
    }
}

impl Session {
    fn enforce(&mut self, authorized_model: &str) -> bool {
        let mut changed = false;
        if let Slot::Object(transcription) = &mut self.input_audio_transcription {
            changed |= transcription.set_model(authorized_model);
        }
        let Slot::Object(audio) = &mut self.audio else {
            return changed;
        };
        let Slot::Object(input) = &mut audio.input else {
            return changed;
        };
        let Slot::Object(transcription) = &mut input.transcription else {
            return changed;
        };
        changed | transcription.set_model(authorized_model)
    }
}

pub fn enforce_session_update<ConfigError, CallbackError>(
    message: &str,
    authorized_model: &str,
    is_transcription: &mut bool,
) -> Result<String, RealtimeSenderError<ConfigError, CallbackError>>
where
    ConfigError: Error + 'static,
    CallbackError: Error + 'static,
{
    if nests_deeper_than(message, MAX_NESTING) {
        return Err(RealtimeSenderError::SessionUpdateTooDeep);
    }
    match serde_json::from_str::<Json>(message) {
        Ok(Json::Object(_)) => {}
        Ok(_) => return Err(RealtimeSenderError::SessionUpdateNotObject),
        // Forwarding a frame Python decodes would carry the client's model past the rewrite below
        Err(_) => {
            let decoded = python_json::normalize_extensions(message)
                .and_then(|normalized| serde_json::from_str::<Json>(&normalized).ok());
            return match decoded {
                Some(Json::Object(_)) => Err(RealtimeSenderError::SessionUpdateUnreadable),
                Some(_) => Err(RealtimeSenderError::SessionUpdateNotObject),
                None => Ok(message.to_owned()),
            };
        }
    }
    let envelope = serde_json::from_str::<Envelope>(message)
        .map_err(|source| RealtimeSenderError::SessionUpdateInvalid { source })?;
    if !matches!(
        (envelope.kind, envelope.session),
        (Slot::Object(_), Slot::Object(_))
    ) {
        return Ok(message.to_owned());
    }
    let mut update = serde_json::from_str::<Update>(message)
        .map_err(|source| RealtimeSenderError::SessionUpdateInvalid { source })?;
    if matches!(
        &update.session.r#type,
        Slot::Object(SessionType::Transcription)
    ) {
        *is_transcription = true;
    }
    if !update.session.enforce(authorized_model) {
        return Ok(message.to_owned());
    }
    serde_json::to_string(&update).map_err(|source| RealtimeSenderError::Encode { source })
}

fn nests_deeper_than(message: &str, limit: usize) -> bool {
    let mut depth = 0_usize;
    let mut in_string = false;
    let mut escaped = false;
    for byte in message.bytes() {
        if in_string {
            if byte == b'\\' && !escaped {
                escaped = true;
                continue;
            }
            if byte == b'"' && !escaped {
                in_string = false;
            }
            escaped = false;
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' | b'[' => {
                if depth == limit {
                    return true;
                }
                depth = depth.saturating_add(1);
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    false
}
