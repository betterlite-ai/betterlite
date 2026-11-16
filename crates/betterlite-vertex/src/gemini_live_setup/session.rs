use betterlite_prompts::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};
use serde_json::Number;

use super::turn_detection::TurnDetection;

#[derive(Clone, Debug, Default, Deserialize)]
pub struct SessionUpdate {
    pub instructions: Option<String>,
    pub temperature: Option<Number>,
    pub max_response_output_tokens: Option<MaxResponseTokens>,
    pub modalities: Option<Vec<String>>,
    pub tools: Option<Vec<Object>>,
    pub input_audio_transcription: Option<Json>,
    pub turn_detection: Option<TurnDetection>,
    pub voice: Option<String>,
    pub output_modalities: Option<Vec<String>>,
    pub audio: Option<SessionAudio>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct SessionAudio {
    pub input: Option<SessionAudioInput>,
    pub output: Option<SessionAudioOutput>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct SessionAudioInput {
    pub transcription: Option<Json>,
    pub turn_detection: Option<TurnDetection>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct SessionAudioOutput {
    pub voice: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum MaxResponseTokens {
    Count(i64),
    Infinite(Infinite),
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Infinite {
    Inf,
}

#[must_use]
pub fn normalize_session(session: &SessionUpdate) -> SessionUpdate {
    let mut normalized = session.clone();
    if normalized.modalities.is_none() {
        normalized.modalities = normalized.output_modalities.clone();
    }
    if let Some(audio) = normalized.audio.clone() {
        let SessionAudio { input, output } = audio;
        if normalized.input_audio_transcription.is_none()
            && let Some(input) = &input
        {
            normalized.input_audio_transcription = input.transcription.clone();
        }
        if normalized.turn_detection.is_none()
            && let Some(input) = &input
        {
            normalized.turn_detection = input.turn_detection.clone();
        }
        if let Some(voice) = output.and_then(|output| output.voice)
            && !voice.is_empty()
        {
            normalized.voice = Some(voice);
        }
    }
    normalized
}
