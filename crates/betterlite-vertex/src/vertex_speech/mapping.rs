use betterlite_prompts::arguments::json::{Json, Object};
use serde::{Deserialize, Serialize};

pub const DEFAULT_LANGUAGE_CODE: &str = "en-US";
pub const DEFAULT_VOICE_NAME: &str = "en-US-Studio-O";
pub const DEFAULT_AUDIO_ENCODING: &str = "LINEAR16";
pub const DEFAULT_SPEAKING_RATE: &str = "1";

// LiteLLM's VOICE_MAPPINGS: an OpenAI voice name becomes a Cloud TTS voice
pub const VOICE_MAPPINGS: [(&str, &str); 6] = [
    ("alloy", "en-US-Studio-O"),
    ("echo", "en-US-Studio-M"),
    ("fable", "en-GB-Studio-B"),
    ("onyx", "en-US-Wavenet-D"),
    ("nova", "en-US-Studio-O"),
    ("shimmer", "en-US-Wavenet-F"),
];

// LiteLLM's FORMAT_MAPPINGS; the provider has no AAC, so that format asks for MP3
pub const FORMAT_MAPPINGS: [(&str, &str); 6] = [
    ("mp3", "MP3"),
    ("opus", "OGG_OPUS"),
    ("aac", "MP3"),
    ("flac", "FLAC"),
    ("wav", "LINEAR16"),
    ("pcm", "LINEAR16"),
];

// the request's voice is a name, an already built Vertex voice record, or nothing
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum SpeechVoice {
    Name(String),
    Options(Object),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct SpeechOptionalParams {
    #[serde(default)]
    pub response_format: Option<String>,
    #[serde(default)]
    pub speed: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct SpeechKwargs {
    #[serde(default)]
    pub audio_config: Option<SpeechAudioConfig>,
    #[serde(default)]
    pub use_ssml: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SpeechAudioConfig {
    #[serde(
        rename = "audioEncoding",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub audio_encoding: Option<String>,
    #[serde(
        rename = "speakingRate",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub speaking_rate: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpeechMapping {
    pub voice: Option<String>,
    pub voice_dict: Option<Object>,
    pub audio_encoding: String,
    pub speaking_rate: Option<String>,
    pub audio_config: Option<SpeechAudioConfig>,
    pub use_ssml: bool,
}

#[must_use]
pub fn map_openai_params(
    voice: Option<&SpeechVoice>,
    optional_params: &SpeechOptionalParams,
    kwargs: &SpeechKwargs,
) -> SpeechMapping {
    let (mapped_voice, voice_dict) = map_voice(voice);
    let audio_encoding = optional_params.response_format.as_deref().map_or_else(
        || DEFAULT_AUDIO_ENCODING.to_owned(),
        |format| {
            FORMAT_MAPPINGS
                .iter()
                .find_map(|(name, encoding)| (*name == format).then_some(*encoding))
                .map_or_else(|| format.to_owned(), str::to_owned)
        },
    );
    SpeechMapping {
        voice: mapped_voice,
        voice_dict,
        audio_encoding,
        speaking_rate: optional_params.speed.map(|speed| speed.to_string()),
        audio_config: kwargs.audio_config.clone(),
        use_ssml: kwargs.use_ssml,
    }
}

fn map_voice(voice: Option<&SpeechVoice>) -> (Option<String>, Option<Object>) {
    match voice {
        None => (None, None),
        Some(SpeechVoice::Options(options)) => (None, Some(options.clone())),
        Some(SpeechVoice::Name(name)) => {
            let mapped: &str = VOICE_MAPPINGS
                .iter()
                .find_map(|(source, target)| (*source == name.as_str()).then_some(*target))
                .unwrap_or(name);
            let mut settings = Object::new();
            settings.insert(
                "languageCode".to_owned(),
                Json::Text(voice_language(mapped)),
            );
            settings.insert("name".to_owned(), Json::Text(mapped.to_owned()));
            (Some(name.clone()), Some(settings))
        }
    }
}

// the language code is the first two hyphen components of the mapped voice name
pub(crate) fn voice_language(name: &str) -> String {
    name.split_once('-').map_or_else(
        || DEFAULT_LANGUAGE_CODE.to_owned(),
        |(language, region)| {
            let region = region.split('-').next().unwrap_or_default();
            format!("{language}-{region}")
        },
    )
}
