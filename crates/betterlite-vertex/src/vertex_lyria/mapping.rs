use betterlite_models::metadata::identity::AudioFormat;
use betterlite_prompts::arguments::json::{Json, Object};

use crate::model_policy::lyria::LyriaOverlay;
use crate::vertex_speech::mapping::SpeechVoice;

use super::error::VertexLyriaError;

// LiteLLM's Lyria config advertises only the parameter its two APIs can honour
pub const SUPPORTED_OPENAI_PARAMS: [&str; 1] = ["response_format"];

// LiteLLM checks these two in this order and names them in this order
const UNSUPPORTED_PARAMS: [&str; 2] = ["speed", "instructions"];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DropParams {
    pub call: bool,
    pub global: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LyriaMapping {
    pub voice: Option<String>,
    pub params: Object,
}

pub fn map_openai_params(
    overlay: &LyriaOverlay<'_>,
    model: &str,
    optional_params: &Object,
    voice: Option<&SpeechVoice>,
    drop: DropParams,
) -> Result<LyriaMapping, VertexLyriaError> {
    let model_info = overlay
        .model_info(model)
        .ok_or_else(|| VertexLyriaError::UnknownModel {
            model: model.to_owned(),
        })?;
    let base_model = model.strip_prefix("vertex_ai/").unwrap_or(model);
    let mut params = optional_params.clone();
    let unsupported: Vec<&str> = UNSUPPORTED_PARAMS
        .into_iter()
        .filter(|param| {
            params
                .get(*param)
                .is_some_and(|value| !matches!(value, Json::Null))
        })
        .collect();
    if !unsupported.is_empty() {
        if !drop.call && !drop.global {
            return Err(VertexLyriaError::UnsupportedParams {
                model: base_model.to_owned(),
                params: unsupported.join(", "),
            });
        }
        for param in &unsupported {
            params.shift_remove(*param);
        }
    }
    let mut supported: Vec<&str> = model_info
        .supported_audio_formats
        .iter()
        .map(|format| format_name(*format))
        .collect();
    supported.sort_unstable();
    if let Some(value) = params
        .get("response_format")
        .filter(|value| !matches!(value, Json::Null))
        && !value
            .as_str()
            .is_some_and(|format| supported.contains(&format))
    {
        if !drop.call && !drop.global {
            return Err(VertexLyriaError::UnsupportedFormat {
                model: base_model.to_owned(),
                format: format_text(value),
                supported: supported.join(", "),
            });
        }
        params.shift_remove("response_format");
    }
    Ok(LyriaMapping {
        voice: match voice {
            Some(SpeechVoice::Name(name)) => Some(name.clone()),
            Some(SpeechVoice::Options(_)) | None => None,
        },
        params,
    })
}

const fn format_name(format: AudioFormat) -> &'static str {
    match format {
        AudioFormat::Mp3 => "mp3",
        AudioFormat::Wav => "wav",
    }
}

// LiteLLM prints the rejected value with repr, which docs/compatibility.md leaves to the runtime
fn format_text(value: &Json) -> String {
    match value {
        Json::Text(text) => format!("'{text}'"),
        Json::Null => "null".to_owned(),
        Json::Bool(flag) => flag.to_string(),
        Json::Number(number) => number.to_string(),
        Json::List(_) => "a list".to_owned(),
        Json::Object(_) => "an object".to_owned(),
    }
}
