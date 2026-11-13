use serde::{Deserialize, Serialize};

use super::error::GeminiThinkingError;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct AudioParam {
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub voice: Option<String>,
    #[serde(default)]
    pub language_code: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeechConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice_config: Option<VoiceConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language_code: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceConfig {
    pub prebuilt_voice_config: PrebuiltVoiceConfig,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrebuiltVoiceConfig {
    pub voice_name: String,
}

pub fn map_audio_params(audio: &AudioParam) -> Result<SpeechConfig, GeminiThinkingError> {
    if let Some(format) = audio.format.as_deref()
        && format != "pcm16"
    {
        return Err(GeminiThinkingError::UnsupportedAudioFormat {
            format: format.to_owned(),
        });
    }
    let voice_config = audio.voice.as_ref().map(|voice| VoiceConfig {
        prebuilt_voice_config: PrebuiltVoiceConfig {
            voice_name: voice.clone(),
        },
    });
    Ok(SpeechConfig {
        voice_config,
        language_code: audio.language_code.clone(),
    })
}
