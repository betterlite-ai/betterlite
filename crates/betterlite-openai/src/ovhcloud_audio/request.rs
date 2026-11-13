use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;

use crate::api::audio_file_input::ProcessedAudioFile;
use crate::openai_base_config::{Headers, Object};
use crate::ovhcloud_chat::DEFAULT_API_BASE;

use super::TRANSCRIPTION_ENDPOINT;

// OVHCloud implements the OpenAI-compatible Whisper interface, so the OpenAI audio params pass through in this order
pub const SUPPORTED_OPENAI_PARAMS: [&str; 5] = [
    "language",
    "prompt",
    "response_format",
    "timestamp_granularities",
    "temperature",
];

#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptionRequest {
    pub fields: Vec<(String, Value)>,
    pub file: ProcessedAudioFile,
}

#[must_use]
pub fn complete_url(api_base: Option<&str>) -> String {
    format!(
        "{}/{TRANSCRIPTION_ENDPOINT}",
        api_base.map_or(DEFAULT_API_BASE, |base| base.trim_end_matches('/'))
    )
}

// the key is the explicit api_key else the caller-read OVHCLOUD_API_KEY, and a missing one still writes Bearer None
#[must_use]
pub fn validate_environment(
    headers: &Headers,
    api_key: Option<&SecretString>,
    env_api_key: Option<&SecretString>,
) -> Headers {
    let mut defaults = Headers::new();
    let key = api_key.or(env_api_key);
    defaults.insert(
        String::from("Authorization"),
        key.map_or_else(
            || String::from("Bearer None"),
            |secret| format!("Bearer {}", secret.expose_secret()),
        ),
    );
    defaults.insert(String::from("accept"), String::from("application/json"));
    defaults.extend(headers.clone());
    defaults
}

pub fn map_openai_params(non_default_params: &Object, optional_params: &mut Object) {
    for (name, value) in non_default_params {
        if SUPPORTED_OPENAI_PARAMS.contains(&name.as_str()) {
            optional_params.insert(name.clone(), value.clone());
        }
    }
}

// the form carries model first, then each supported param that is set, and the file rides beside it as (name, content, type)
#[must_use]
pub fn transcription_request(
    model: &str,
    audio: &ProcessedAudioFile,
    optional_params: &Object,
) -> TranscriptionRequest {
    let mut fields = vec![(String::from("model"), Value::String(model.to_owned()))];
    fields.extend(
        SUPPORTED_OPENAI_PARAMS
            .iter()
            .filter_map(|name| optional_params.get(*name).map(|value| (*name, value)))
            .filter(|(_, value)| !value.is_null())
            .map(|(name, value)| (String::from(name), value.clone())),
    );
    TranscriptionRequest {
        fields,
        file: audio.clone(),
    }
}
