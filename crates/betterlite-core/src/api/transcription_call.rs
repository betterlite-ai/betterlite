pub mod call;
pub mod dispatch;
pub mod error;
pub mod plan;
pub mod service;
pub mod target;

mod tests;

use betterlite_context::media_config_selection::audio_transcription::AudioTranscriptionConfigTag;
use betterlite_context::non_default_params::names::is_openai_compatible_provider;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::api::transcription_contract::TranscriptionResponse;
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;
use serde_json::Number;

use self::error::TranscriptionError;

// transcription's timeout default, and OpenAI's own retry default, which litellm.num_retries never replaces
pub const DEFAULT_TIMEOUT_SECONDS: f64 = 600.0;
pub const DEFAULT_MAX_RETRIES: u32 = 2;

// has_native_transcription_endpoint is true only for these two provider configs
const NATIVE_ENDPOINT_CONFIGS: [AudioTranscriptionConfigTag; 2] = [
    AudioTranscriptionConfigTag::XAI,
    AudioTranscriptionConfigTag::EdenAI,
];

#[must_use]
pub fn is_native_endpoint(config: Option<AudioTranscriptionConfigTag>) -> bool {
    match config {
        Some(tag) => NATIVE_ENDPOINT_CONFIGS.contains(&tag),
        None => false,
    }
}

// OPENAI_AUDIO_TRANSCRIPTION_PROVIDERS: "openai" plus every openai-compatible provider
#[must_use]
pub fn is_openai_audio_provider(provider: &str) -> bool {
    match provider {
        "openai" => true,
        other => is_openai_compatible_provider(other),
    }
}

// TranscriptionResponse(**cached_dict): the typed constructor, never a second provider decoder
pub fn cached_audio_response(value: Json) -> Result<TranscriptionResponse, TranscriptionError> {
    let Json::Object(fields) = value else {
        return Err(TranscriptionError::CachedResponseNotObject);
    };
    let mut extra = Object::new();
    let mut text = None;
    for (name, value) in fields {
        if name == "text" {
            text = match value {
                Json::Null => None,
                Json::Text(text) => Some(text),
                Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
                    return Err(TranscriptionError::CachedField {
                        field: String::from("text"),
                    });
                }
            };
            continue;
        }
        let converted = serde_json::to_value(&value)
            .map_err(|source| TranscriptionError::CachedValue { source })?;
        extra.insert(name, converted);
    }
    Ok(TranscriptionResponse { text, extra })
}

// type(response)'s text in the final wrapper check
#[must_use]
pub fn error_type_name(value: &Json) -> &'static str {
    match value {
        Json::Null => "<class 'NoneType'>",
        Json::Bool(_) => "<class 'bool'>",
        Json::Number(number) if number.is_i64() || number.is_u64() => "<class 'int'>",
        Json::Number(_) => "<class 'float'>",
        Json::Text(_) => "<class 'str'>",
        Json::List(_) => "<class 'list'>",
        Json::Object(_) => "<class 'dict'>",
    }
}

// LiteLLM stores the duration under _hidden_params, merging into any hidden params a provider set
pub(crate) fn store_hidden_duration(response: &mut TranscriptionResponse, duration: Number) {
    let hidden = response
        .extra
        .entry(String::from("_hidden_params"))
        .or_insert_with(|| serde_json::to_value(Json::Object(IndexMap::new())).unwrap_or_default());
    let Some(fields) = hidden.as_object_mut() else {
        return;
    };
    fields.insert(
        String::from("audio_transcription_duration"),
        serde_json::to_value(Json::Number(duration)).unwrap_or_default(),
    );
}

// a public duration that is present and not null suppresses the fallback, including an explicit zero
pub(crate) fn public_duration_is_absent(response: &TranscriptionResponse) -> bool {
    response
        .extra
        .get("duration")
        .is_none_or(|value| value.is_null())
}

pub(crate) fn decisions_json(value: &JsonValue) -> Json {
    match value {
        JsonValue::Null => Json::Null,
        JsonValue::Bool(flag) => Json::Bool(*flag),
        JsonValue::Number(number) => Json::Number(number.clone()),
        JsonValue::Text(text) => Json::Text(text.clone()),
        JsonValue::Array(items) => Json::List(items.iter().map(decisions_json).collect()),
        JsonValue::Object(fields) => Json::Object(
            fields
                .iter()
                .map(|(name, value)| (name.clone(), decisions_json(value)))
                .collect(),
        ),
    }
}
