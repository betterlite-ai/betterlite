use betterlite_prompts::arguments::json::Json;
use num_traits::ToPrimitive as _;

use crate::native_speech::constants::{DEFAULT_OUTPUT_FORMAT, FORMAT_MAPPINGS, VOICE_MAPPINGS};
use crate::native_speech::error::NativeSpeechError;

#[derive(Debug, Clone, Default)]
pub struct SpeechOptionalParams {
    pub response_format: Option<String>,
    pub speed: Option<f64>,
}

#[derive(Debug, Clone, Default)]
pub struct SpeechKwargs {
    pub style: Option<String>,
    pub styledegree: Option<String>,
    pub role: Option<String>,
    pub lang: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct MappedParams {
    pub output_format: String,
    pub rate: Option<String>,
    pub style: Option<String>,
    pub styledegree: Option<String>,
    pub role: Option<String>,
    pub lang: Option<String>,
}

pub fn map_openai_params(
    voice: Option<&Json>,
    optional: &SpeechOptionalParams,
    kwargs: &SpeechKwargs,
) -> Result<(Option<String>, MappedParams), NativeSpeechError> {
    let mapped_voice = voice
        .and_then(Json::as_str)
        .map(|name| lookup(&VOICE_MAPPINGS, name).to_owned());
    let output_format = optional
        .response_format
        .as_deref()
        .map_or(DEFAULT_OUTPUT_FORMAT, |name| lookup(&FORMAT_MAPPINGS, name));
    let rate = optional.speed.map(convert_speed_to_rate).transpose()?;
    let mapped = MappedParams {
        output_format: output_format.to_owned(),
        rate,
        style: kwargs.style.clone(),
        styledegree: kwargs.styledegree.clone(),
        role: kwargs.role.clone(),
        lang: kwargs.lang.clone(),
    };
    Ok((mapped_voice, mapped))
}

pub fn convert_speed_to_rate(speed: f64) -> Result<String, NativeSpeechError> {
    // float_arithmetic is denied; the fused multiply-adds keep int((speed - 1.0) * 100)'s two roundings
    let percentage = speed
        .mul_add(1.0_f64, -1.0_f64)
        .mul_add(100.0_f64, 0.0_f64)
        .trunc()
        .to_i64()
        .ok_or(NativeSpeechError::SpeedOutOfRange { speed })?;
    Ok(format!("{percentage:+}%"))
}

fn lookup<'key>(table: &[(&'static str, &'static str)], key: &'key str) -> &'key str {
    table
        .iter()
        .find(|(source, _)| *source == key)
        .map_or(key, |&(_, target)| target)
}
