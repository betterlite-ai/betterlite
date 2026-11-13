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
const DEFAULT_SAMPLE_RATE: i64 = 32000;
const DEFAULT_BITRATE: i64 = 128000;
const DEFAULT_CHANNEL: i64 = 1;
const DEFAULT_FORMAT: &str = "mp3";

// MiniMax's base carries no version, unlike its chat base that already ends in /v1
#[must_use]
pub fn get_complete_url(api_base: Option<&str>, environment_base: Option<&str>) -> String {
    format!(
        "{}{TTS_ENDPOINT_PATH}",
        api_base
            .or(environment_base)
            .unwrap_or(TTS_BASE_URL)
            .trim_end_matches('/')
    )
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

#[must_use]
pub fn speech_request_body(
    model: &str,
    text: &str,
    voice: &str,
    optional_params: &Object,
) -> Object {
    let mut params = optional_params.clone();
    let voice_id = take(
        &mut params,
        "voice_id",
        Value::String(
            if voice.is_empty() {
                DEFAULT_VOICE_ID
            } else {
                voice
            }
            .to_owned(),
        ),
    );
    let speed = take(&mut params, "speed", Value::from(DEFAULT_SPEED));
    let format = take(&mut params, "format", Value::from(DEFAULT_FORMAT));
    let vol = take(&mut params, "vol", Value::from(DEFAULT_VOL));
    let pitch = take(&mut params, "pitch", Value::from(DEFAULT_PITCH));
    let sample_rate = take(&mut params, "sample_rate", Value::from(DEFAULT_SAMPLE_RATE));
    let bitrate = take(&mut params, "bitrate", Value::from(DEFAULT_BITRATE));
    let channel = take(&mut params, "channel", Value::from(DEFAULT_CHANNEL));
    let output_format = take(
        &mut params,
        "output_format",
        Value::from(DEFAULT_OUTPUT_FORMAT),
    );
    let voice_setting = Object::from_iter([
        (String::from("voice_id"), voice_id),
        (String::from("speed"), speed),
        (String::from("vol"), vol),
        (String::from("pitch"), pitch),
    ]);
    let audio_setting = Object::from_iter([
        (String::from("sample_rate"), sample_rate),
        (String::from("bitrate"), bitrate),
        (String::from("format"), format),
        (String::from("channel"), channel),
    ]);
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
