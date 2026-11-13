use betterlite_config::sections::mistral_resources::Settings;
use betterlite_http::proxy::UpstreamResponse;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;

use super::error::MistralAudioError;
use super::reply::reply_fields;
use crate::api::audio_file_input::ProcessedAudioFile;
use crate::api::transcription_contract::TranscriptionResponse;
use crate::openai_base_config::Object;

pub const DEFAULT_BASE: &str = "https://api.mistral.ai/v1";
pub const OPENAI_PARAMS: [&str; 4] = [
    "language",
    "temperature",
    "timestamp_granularities",
    "response_format",
];
// LiteLLM keeps these names out of the provider extras because they were handled already or are internal
const RESERVED_PARAMS: [&str; 2] = ["model", "OPENAI_TRANSCRIPTION_PARAMS"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptionRequest {
    pub form: Object,
    pub file: ProcessedAudioFile,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoxtralTranscription {
    pub response: TranscriptionResponse,
    pub hidden: Object,
}
pub fn map_openai_params(non_default_params: &Object, optional_params: &mut Object) {
    for (name, value) in non_default_params {
        if OPENAI_PARAMS.contains(&name.as_str()) {
            optional_params.insert(name.clone(), value.clone());
        }
    }
}
#[must_use]
pub fn validate_environment(
    headers: &IndexMap<String, String>,
    api_key: Option<&SecretString>,
    settings: &Settings,
) -> IndexMap<String, String> {
    let resolved = api_key.or(settings.api_key.as_ref());
    let rendered = resolved.map_or_else(
        || String::from("None"),
        |key| key.expose_secret().to_owned(),
    );
    let mut merged = IndexMap::from([
        (String::from("Authorization"), format!("Bearer {rendered}")),
        (String::from("accept"), String::from("application/json")),
    ]);
    merged.extend(
        headers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone())),
    );
    merged
}
#[must_use]
pub fn complete_url(api_base: Option<&str>) -> String {
    format!(
        "{}/audio/transcriptions",
        api_base.map_or(DEFAULT_BASE, |base| base.trim_end_matches('/'))
    )
}
pub fn transcription_request(
    model: &str,
    audio: &ProcessedAudioFile,
    optional_params: &Object,
) -> Result<TranscriptionRequest, MistralAudioError> {
    let mut form = Object::new();
    form.insert(String::from("model"), Value::String(model.to_owned()));
    for name in OPENAI_PARAMS {
        if let Some(value) = optional_params.get(name).filter(|value| !value.is_null()) {
            form.insert(name.to_owned(), value.clone());
        }
    }
    for (name, value) in optional_params {
        if value.is_null()
            || OPENAI_PARAMS.contains(&name.as_str())
            || RESERVED_PARAMS.contains(&name.as_str())
        {
            continue;
        }
        // a boolean is lowercased before it is sent; only a scalar keeps Python's str() spelling
        let text = match value {
            Value::Bool(true) => String::from("true"),
            Value::Bool(false) => String::from("false"),
            Value::String(text) => text.clone(),
            Value::Number(number) => number.to_string(),
            Value::Null | Value::Array(_) | Value::Object(_) => {
                return Err(MistralAudioError::FormValue {
                    field: name.to_owned(),
                });
            }
        };
        form.insert(name.clone(), Value::String(text));
    }
    Ok(TranscriptionRequest {
        form,
        file: audio.clone(),
    })
}
pub fn decode_voxtral_transcript(
    response: &UpstreamResponse,
) -> Result<VoxtralTranscription, MistralAudioError> {
    let fields = reply_fields(response)?;
    let text = match fields.get("text") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Null) | None => String::new(),
        Some(_) => return Err(MistralAudioError::TextNotString),
    };
    let mut extra = Object::new();
    for name in ["segments", "language"] {
        if let Some(value) = fields.get(name) {
            extra.insert(name.to_owned(), value.clone());
        }
    }
    Ok(VoxtralTranscription {
        response: TranscriptionResponse {
            text: Some(text),
            extra,
        },
        hidden: fields,
    })
}
