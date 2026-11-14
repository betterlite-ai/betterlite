use betterlite_prompts::arguments::json::Object;
use betterlite_vertex::vertex_speech::mapping::{SpeechAudioConfig, SpeechVoice};
use rust_decimal::Decimal;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    AlloyVoice { voices: Vec<Option<SpeechVoice>> },
    CustomVoiceLanguage { voices: Vec<Option<SpeechVoice>> },
    AacMapsMp3 { formats: Vec<Option<String>> },
    SsmlSubstring { cases: Vec<Case> },
    FullAudioConfig { cases: Vec<Case> },
    VoiceDictPrecedence { cases: Vec<Case> },
    DecodedStatus200 { bodies: Vec<String> },
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub(crate) struct Case {
    pub(crate) input: Option<String>,
    pub(crate) use_ssml: bool,
    pub(crate) response_format: Option<String>,
    pub(crate) speed: Option<Decimal>,
    pub(crate) audio_config: Option<SpeechAudioConfig>,
    pub(crate) litellm_voice: Option<Object>,
    pub(crate) mapped_voice: Option<Object>,
    pub(crate) voice: Option<String>,
}
