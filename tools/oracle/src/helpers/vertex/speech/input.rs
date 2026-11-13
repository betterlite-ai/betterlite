use betterlite_prompts::arguments::json::Object;
use betterlite_vertex::vertex_speech::mapping::{SpeechAudioConfig, SpeechVoice};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    AlloyVoice {
        voices: Vec<Option<SpeechVoice>>,
    },
    CustomVoiceLanguage {
        voices: Vec<String>,
    },
    AacMapsMp3 {
        formats: Vec<Option<String>>,
    },
    SsmlSubstring {
        cases: Vec<SsmlCase>,
    },
    FullAudioConfig {
        cases: Vec<AudioCase>,
    },
    VoiceDictPrecedence {
        cases: Vec<VoiceDictCase>,
    },
    DecodedStatus200 {
        bodies: Vec<String>,
    },
}

#[derive(Debug, Deserialize)]
pub(crate) struct SsmlCase {
    pub(crate) input: String,
    #[serde(default)]
    pub(crate) use_ssml: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AudioCase {
    #[serde(default)]
    pub(crate) response_format: Option<String>,
    #[serde(default)]
    pub(crate) speed: Option<i64>,
    #[serde(default)]
    pub(crate) audio_config: Option<SpeechAudioConfig>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct VoiceDictCase {
    #[serde(default)]
    pub(crate) litellm_voice: Option<Object>,
    #[serde(default)]
    pub(crate) mapped_voice: Option<Object>,
    #[serde(default)]
    pub(crate) voice: Option<String>,
}
