use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;

use crate::api::audio_file_input::ProcessedAudioFile;
use crate::openai_base_config::{Headers, Object};

use super::TRANSCRIPTION_ENDPOINT;
use super::error::{ScalewayAudioError, missing_key};

pub const DEFAULT_API_BASE: &str = "https://api.scaleway.ai/v1";

pub const SUPPORTED_OPENAI_PARAMS: [&str; 5] = [
    "language",
    "prompt",
    "response_format",
    "temperature",
    "timestamp_granularities",
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

// an empty key counts as missing, whether the caller passed it or read an empty SCW_SECRET_KEY
pub fn validate_environment(
    headers: &Headers,
    api_key: Option<&SecretString>,
    env_api_key: Option<&SecretString>,
) -> Result<Headers, ScalewayAudioError> {
    let key = api_key
        .or(env_api_key)
        .filter(|secret| !secret.expose_secret().is_empty())
        .ok_or_else(missing_key)?;
    let mut defaults = Headers::from([
        (
            String::from("Authorization"),
            format!("Bearer {}", key.expose_secret()),
        ),
        (String::from("accept"), String::from("application/json")),
    ]);
    defaults.extend(headers.clone());
    Ok(defaults)
}

pub fn map_openai_params(non_default_params: &Object, optional_params: &mut Object) {
    for (name, value) in non_default_params {
        if SUPPORTED_OPENAI_PARAMS.contains(&name.as_str()) {
            optional_params.insert(name.clone(), value.clone());
        }
    }
}

// model opens the form, then every supported param that is not null, in the supported order; the file rides beside it
#[must_use]
pub fn transcription_request(
    model: &str,
    audio: &ProcessedAudioFile,
    optional_params: &Object,
) -> TranscriptionRequest {
    let mut fields = vec![(String::from("model"), Value::String(model.to_owned()))];
    for name in SUPPORTED_OPENAI_PARAMS {
        match optional_params.get(name) {
            None | Some(Value::Null) => {}
            Some(value) => fields.push((String::from(name), value.clone())),
        }
    }
    TranscriptionRequest {
        fields,
        file: audio.clone(),
    }
}
