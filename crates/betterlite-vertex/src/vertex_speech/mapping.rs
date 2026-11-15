use betterlite_prompts::arguments::json::{Json, Object};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

pub const DEFAULT_LANGUAGE_CODE: &str = "en-US";
pub const DEFAULT_VOICE_NAME: &str = "en-US-Studio-O";
pub const DEFAULT_AUDIO_ENCODING: &str = "LINEAR16";
pub const DEFAULT_SPEAKING_RATE: &str = "1";

pub const VOICE_MAPPINGS: [(&str, &str); 6] = [
    ("alloy", "en-US-Studio-O"),
    ("echo", "en-US-Studio-M"),
    ("fable", "en-GB-Studio-B"),
    ("onyx", "en-US-Wavenet-D"),
    ("nova", "en-US-Studio-O"),
    ("shimmer", "en-US-Wavenet-F"),
];

// the provider has no AAC, so that format asks for MP3
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpeechOptionalParams {
    pub response_format: Option<String>,
    pub speed: Option<Decimal>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpeechKwargs {
    pub audio_config: Option<SpeechAudioConfig>,
    pub use_ssml: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct SpeechAudioConfig {
    #[serde(rename = "audioEncoding", skip_serializing_if = "Option::is_none")]
    pub audio_encoding: Option<String>,
    #[serde(rename = "speakingRate", skip_serializing_if = "Option::is_none")]
    pub speaking_rate: Option<String>,
    #[serde(flatten)]
    pub other: Object,
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
    let (voice_name, voice_dict) = map_voice(voice);
    let encoding = optional_params
        .response_format
        .as_deref()
        .map_or(DEFAULT_AUDIO_ENCODING, |format| {
            lookup(&FORMAT_MAPPINGS, format)
        });
    SpeechMapping {
        voice: voice_name,
        voice_dict,
        audio_encoding: encoding.to_owned(),
        speaking_rate: optional_params.speed.map(|speed| speed.to_string()),
        audio_config: kwargs.audio_config.clone(),
        use_ssml: kwargs.use_ssml,
    }
}

// an unmapped key passes through unchanged
fn lookup<'key>(table: &[(&'static str, &'static str)], key: &'key str) -> &'key str {
    table
        .iter()
        .find(|(source, _)| *source == key)
        .map_or(key, |&(_, target)| target)
}

fn map_voice(voice: Option<&SpeechVoice>) -> (Option<String>, Option<Object>) {
    match voice {
        None => (None, None),
        Some(SpeechVoice::Options(options)) => (None, Some(options.clone())),
        Some(SpeechVoice::Name(name)) => {
            let mapped = lookup(&VOICE_MAPPINGS, name);
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
        |(language, tail)| {
            let region = tail.split('-').next().unwrap_or_default();
            format!("{language}-{region}")
        },
    )
}
