use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;

use crate::openai_base_config::{Headers, Object};

use super::error::MinimaxSpeechError;
use super::params::DEFAULT_VOICE_ID;

pub const TTS_BASE_URL: &str = "https://api.minimax.io";
const TTS_ENDPOINT_PATH: &str = "/v1/t2a_v2";
const DEFAULT_OUTPUT_FORMAT: &str = "hex";
const DEFAULT_SPEED: f64 = 1.0;
const DEFAULT_VOL: f64 = 1.0;
const DEFAULT_PITCH: i64 = 0;
const DEFAULT_SAMPLE_RATE: i64 = 32_000;
const DEFAULT_BITRATE: i64 = 128_000;
const DEFAULT_CHANNEL: i64 = 1;
const DEFAULT_FORMAT: &str = "mp3";

// MiniMax's base carries no version, unlike its chat base that already ends in /v1
#[must_use]
pub fn get_complete_url(api_base: Option<&str>, environment_base: Option<&str>) -> String {
    let base = api_base
        .or(environment_base)
        .unwrap_or(TTS_BASE_URL)
        .trim_end_matches('/');
    format!("{base}{TTS_ENDPOINT_PATH}")
}

pub fn validate_environment(
    headers: &Headers,
    api_key: Option<&SecretString>,
    global_api_key: Option<&SecretString>,
    environment_api_key: Option<&SecretString>,
) -> Result<Headers, MinimaxSpeechError> {
    let key = api_key
        .or(global_api_key)
        .or(environment_api_key)
        .ok_or(MinimaxSpeechError::MissingApiKey)?;
    let mut merged = headers.clone();
    merged.insert(
        String::from("Authorization"),
        format!("Bearer {}", key.expose_secret()),
    );
    merged.insert(
        String::from("Content-Type"),
        String::from("application/json"),
    );
    Ok(merged)
}

// an absent or null option takes MiniMax's default; any other value is sent as the caller gave it
fn take(params: &mut Object, name: &str, default: Value) -> Value {
    match params.remove(name) {
        Some(Value::Null) | None => default,
        Some(value) => value,
    }
}

fn take_settings(params: &mut Object, defaults: &[(&str, Value)]) -> Object {
    defaults
        .iter()
        .map(|(name, default)| (String::from(*name), take(params, name, default.clone())))
        .collect()
}

fn build_voice_setting(params: &mut Object, voice: &str) -> Object {
    let fallback = if voice.is_empty() {
        DEFAULT_VOICE_ID
    } else {
        voice
    };
    take_settings(
        params,
        &[
            ("voice_id", Value::String(fallback.to_owned())),
            ("speed", Value::from(DEFAULT_SPEED)),
            ("vol", Value::from(DEFAULT_VOL)),
            ("pitch", Value::from(DEFAULT_PITCH)),
        ],
    )
}

#[must_use]
pub fn speech_request_body(
    model: &str,
    text: &str,
    voice: &str,
    optional_params: &Object,
) -> Object {
    let mut params = optional_params.clone();
    let voice_setting = build_voice_setting(&mut params, voice);
    let audio_setting = take_settings(
        &mut params,
        &[
            ("sample_rate", Value::from(DEFAULT_SAMPLE_RATE)),
            ("bitrate", Value::from(DEFAULT_BITRATE)),
            ("format", Value::from(DEFAULT_FORMAT)),
            ("channel", Value::from(DEFAULT_CHANNEL)),
        ],
    );
    let output_format = take(
        &mut params,
        "output_format",
        Value::from(DEFAULT_OUTPUT_FORMAT),
    );
    let mut body = Object::from_iter([
        (String::from("model"), Value::from(model)),
        (String::from("text"), Value::from(text)),
        (String::from("stream"), Value::Bool(false)),
        (String::from("output_format"), output_format),
        (String::from("voice_setting"), Value::Object(voice_setting)),
        (String::from("audio_setting"), Value::Object(audio_setting)),
    ]);
    merge_extra_body(&mut params, &mut body);
    body
}

fn merge_extra_body(params: &mut Object, body: &mut Object) {
    let Some(Value::Object(extra)) = params.remove("extra_body") else {
        return;
    };
    for (name, value) in extra {
        if !value.is_null() && !body.contains_key(&name) {
            body.insert(name, value);
        }
    }
}
