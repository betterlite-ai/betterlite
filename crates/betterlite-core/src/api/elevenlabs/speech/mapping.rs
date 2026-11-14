use betterlite_context::owned_parameter_names::is_litellm_owned_kwarg;
use betterlite_openai::api::speech_provider_contract::SpeechVoice;
use betterlite_openai::openai_base_config::Object;
use serde_json::{Number, json};

use super::error::SpeechError;
use super::voice::resolve_voice_id;

pub const QUERY_PARAMS_KEY: &str = "__elevenlabs_query_params__";
pub const VOICE_ID_KEY: &str = "__elevenlabs_voice_id__";
const FORMAT_MAPPINGS: [(&str, &str); 3] = [
    ("mp3", "mp3_44100_128"),
    ("pcm", "pcm_44100"),
    ("opus", "opus_48000_128"),
];
const RESERVED_KWARGS: [&str; 8] = [
    QUERY_PARAMS_KEY,
    VOICE_ID_KEY,
    "voice",
    "model",
    "response_format",
    "output_format",
    "extra_body",
    "user",
];

pub struct SpeechMapping {
    pub voice_id: String,
    pub params: Object,
}

fn take_speed(params: &mut Object) -> Option<Number> {
    let speed = params.remove("speed")?;
    speed
        .as_f64()
        .or_else(|| speed.as_str()?.trim().parse::<f64>().ok())
        .or_else(|| speed.as_bool().map(f64::from))
        .and_then(Number::from_f64)
}

// Python copies the options shallowly, so an existing voice_settings object is shared with the caller
fn apply_speed(optional_params: &mut Object, params: &mut Object, speed: &Number) {
    if let Some(settings) = optional_params.get_mut("voice_settings")
        && let Some(shared) = settings.as_object_mut()
    {
        shared.insert("speed".to_owned(), json!(speed));
    }
    if let Some(settings) = params.get_mut("voice_settings")
        && let Some(existing) = settings.as_object_mut()
    {
        existing.insert("speed".to_owned(), json!(speed));
        return;
    }
    let mut settings = Object::new();
    settings.insert("speed".to_owned(), json!(speed));
    params.insert("voice_settings".to_owned(), json!(settings));
}

fn add_specific_params(
    voice_id: &str,
    query: &Object,
    remaining: Object,
    kwargs: &mut Object,
) -> Object {
    let mut mapped: Object = remaining
        .into_iter()
        .filter(|(_, value)| !value.is_null())
        .collect();
    if let Some(extra_body) = kwargs
        .remove("extra_body")
        .and_then(|extra| extra.as_object().cloned())
    {
        mapped.extend(extra_body.into_iter().filter(|(_, value)| !value.is_null()));
    }
    let consumed: Vec<String> = kwargs
        .iter()
        .filter(|(key, value)| {
            !value.is_null()
                && !RESERVED_KWARGS.contains(&key.as_str())
                && !is_litellm_owned_kwarg(key)
        })
        .map(|(key, _)| key.clone())
        .collect();
    for key in consumed {
        if let Some(value) = kwargs.remove(&key) {
            mapped.insert(key, value);
        }
    }
    if query.is_empty() {
        kwargs.remove(QUERY_PARAMS_KEY);
    } else {
        kwargs.insert(QUERY_PARAMS_KEY.to_owned(), json!(query));
    }
    kwargs.insert(VOICE_ID_KEY.to_owned(), json!(voice_id));
    mapped
}

pub fn map_openai_params(
    optional_params: &mut Object,
    voice: Option<&SpeechVoice>,
    kwargs: &mut Object,
) -> Result<SpeechMapping, SpeechError> {
    let mut params = optional_params.clone();
    let voice_id = resolve_voice_id(voice, &mut params)?;
    let mut query = Object::new();
    let requested_format = params
        .remove("response_format")
        .and_then(|format| format.as_str().map(str::to_owned));
    if let Some(requested) = requested_format {
        let mapped = FORMAT_MAPPINGS
            .iter()
            .find(|(name, _)| *name == requested)
            .map_or(requested.as_str(), |(_, format)| *format);
        query.insert("output_format".to_owned(), json!(mapped));
    }
    if let Some(speed) = take_speed(&mut params) {
        apply_speed(optional_params, &mut params, &speed);
    }
    params.remove("instructions");
    let body_params = add_specific_params(&voice_id, &query, params, kwargs);
    Ok(SpeechMapping {
        voice_id,
        params: body_params,
    })
}
