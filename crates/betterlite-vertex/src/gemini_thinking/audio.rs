use betterlite_prompts::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};

use super::error::GeminiThinkingError;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct AudioParam {
    pub format: Option<String>,
    pub voice: Option<String>,
    pub language_code: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ResponseModality {
    Text,
    Image,
    Audio,
    Video,
    #[serde(rename = "MODALITY_UNSPECIFIED")]
    Unspecified,
}

pub fn map_audio_params(audio: &AudioParam) -> Result<Object, GeminiThinkingError> {
    if let Some(format) = audio.format.as_deref()
        && format != "pcm16"
    {
        return Err(GeminiThinkingError::UnsupportedAudioFormat {
            format: format.to_owned(),
        });
    }
    let mut config = Object::new();
    if let Some(voice) = &audio.voice {
        let prebuilt = Object::from([("voiceName".to_owned(), Json::Text(voice.clone()))]);
        let voice_config =
            Object::from([("prebuiltVoiceConfig".to_owned(), Json::Object(prebuilt))]);
        config.insert("voiceConfig".to_owned(), Json::Object(voice_config));
    }
    if let Some(code) = &audio.language_code {
        config.insert("languageCode".to_owned(), Json::Text(code.clone()));
    }
    Ok(config)
}

#[must_use]
pub fn map_response_modalities(modalities: &[String]) -> Vec<ResponseModality> {
    modalities
        .iter()
        .map(|modality| match modality.as_str() {
            "text" => ResponseModality::Text,
            "image" => ResponseModality::Image,
            "audio" => ResponseModality::Audio,
            "video" => ResponseModality::Video,
            _ => ResponseModality::Unspecified,
        })
        .collect()
}
