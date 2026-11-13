use serde_json::Value;

use crate::openai_base_config::Object;

use super::error::MinimaxSpeechError;

pub const SUPPORTED_OPENAI_PARAMS: [&str; 3] = ["voice", "response_format", "speed"];
pub const DEFAULT_VOICE_ID: &str = "male-qn-qingse";
const DEFAULT_SPEED: f64 = 1.0;
const MIN_SPEED: f64 = 0.5;
const MAX_SPEED: f64 = 2.0;

const VOICE_MAPPINGS: [(&str, &str); 6] = [
    ("alloy", "male-qn-qingse"),
    ("echo", "male-qn-jingying"),
    ("fable", "female-shaonv"),
    ("onyx", "male-qn-badao"),
    ("nova", "female-yujie"),
    ("shimmer", "female-tianmei"),
];

const FORMAT_MAPPINGS: [&str; 4] = ["mp3", "pcm", "wav", "flac"];
const DEFAULT_FORMAT: &str = "mp3";

// LiteLLM declares voice as a string or an object naming the voice under one of three keys
pub enum VoiceInput<'voice> {
    Text(&'voice str),
    Fields(&'voice Object),
}

#[derive(Debug)]
pub struct MappedOptions {
    pub voice: String,
    pub params: Object,
}

fn extract_voice_id(voice: &str) -> String {
    let trimmed = voice.trim();
    let normalized = trimmed.to_lowercase();
    for (name, mapped) in VOICE_MAPPINGS {
        if name == normalized.as_str() {
            return mapped.to_owned();
        }
    }
    trimmed.to_owned()
}

fn voice_from_fields(fields: &Object) -> Option<String> {
    for name in ["voice_id", "id", "name"] {
        if let Some(Value::String(candidate)) = fields.get(name)
            && !candidate.trim().is_empty()
        {
            return Some(extract_voice_id(candidate));
        }
    }
    None
}

// A missing voice falls back to the voice_id option and then to MiniMax's default
fn resolve_voice_id(voice: Option<VoiceInput<'_>>, params: &mut Object) -> String {
    let named = match voice {
        Some(VoiceInput::Text(text)) => Some(extract_voice_id(text)),
        Some(VoiceInput::Fields(fields)) => voice_from_fields(fields),
        None => None,
    };
    if let Some(voice_id) = named {
        return voice_id;
    }
    match params.remove("voice_id") {
        Some(Value::String(text)) if !text.trim().is_empty() => extract_voice_id(&text),
        _ => DEFAULT_VOICE_ID.to_owned(),
    }
}

fn mapped_format(response_format: Option<&Value>) -> &'static str {
    let Some(Value::String(name)) = response_format else {
        return DEFAULT_FORMAT;
    };
    for format in FORMAT_MAPPINGS {
        if format == name.as_str() {
            return format;
        }
    }
    DEFAULT_FORMAT
}

fn mapped_speed(speed: Option<Value>) -> Result<f64, MinimaxSpeechError> {
    let Some(speed) = speed else {
        return Ok(DEFAULT_SPEED);
    };
    let Value::Number(number) = speed else {
        return Err(MinimaxSpeechError::Option { name: "speed" });
    };
    let value = number
        .as_f64()
        .ok_or(MinimaxSpeechError::Option { name: "speed" })?;
    Ok(value.clamp(MIN_SPEED, MAX_SPEED))
}

pub fn map_openai_params(
    voice: Option<VoiceInput<'_>>,
    optional_params: &Object,
) -> Result<MappedOptions, MinimaxSpeechError> {
    let mut params = optional_params.clone();
    let mapped_voice = resolve_voice_id(voice, &mut params);
    let response_format = params.remove("response_format");
    if matches!(&response_format, Some(value) if !value.is_null() && !value.is_string()) {
        return Err(MinimaxSpeechError::Option {
            name: "response_format",
        });
    }
    let format = mapped_format(response_format.as_ref());
    let speed = mapped_speed(params.remove("speed"))?;
    params.remove("instructions");
    let mut mapped = Object::new();
    mapped.insert(
        String::from("voice_id"),
        Value::String(mapped_voice.clone()),
    );
    mapped.insert(String::from("format"), Value::from(format));
    mapped.insert(String::from("speed"), Value::from(speed));
    merge_extra_body(&mut params, &mut mapped);
    for (name, value) in params {
        if !value.is_null() {
            mapped.insert(name, value);
        }
    }
    Ok(MappedOptions {
        voice: mapped_voice,
        params: mapped,
    })
}

fn merge_extra_body(params: &mut Object, mapped: &mut Object) {
    let Some(Value::Object(extra)) = params.remove("extra_body") else {
        return;
    };
    for (name, value) in extra {
        if !value.is_null() {
            mapped.insert(name, value);
        }
    }
}
