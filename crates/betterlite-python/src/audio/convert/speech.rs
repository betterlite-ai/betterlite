use super::{ConvertError, field, max_retries, text};
use betterlite_models::decisions::Json;
use betterlite_openai::speech::request::{SpeechFormat, SpeechOptions};
use betterlite_sdk::call_setup::RequestMap;
use indexmap::IndexMap;
use rust_decimal::Decimal;
use std::str::FromStr as _;
use std::time::Duration;

pub(crate) struct SpeechValues {
    pub(crate) model: String,
    pub(crate) input: String,
    pub(crate) voice: Option<String>,
    pub(crate) options: SpeechOptions,
    pub(crate) timeout: Option<Duration>,
    pub(crate) max_retries: Option<u32>,
}

pub(crate) fn speech_values(arguments: &RequestMap) -> Result<SpeechValues, ConvertError> {
    let model = text(arguments, "model")?.ok_or_else(|| field("model", "text"))?;
    let input = text(arguments, "input")?.ok_or_else(|| field("input", "text"))?;
    let voice = text(arguments, "voice")?;
    let response_format = speech_format(text(arguments, "response_format")?.as_deref())?;
    let speed = match arguments.get("speed") {
        Some(Json::Number(value)) => Some(
            Decimal::from_str(&value.to_string()).map_err(|_source| field("speed", "a number"))?,
        ),
        None | Some(Json::Null) => None,
        Some(_other) => return Err(field("speed", "a number")),
    };
    Ok(SpeechValues {
        model,
        input,
        voice,
        options: SpeechOptions {
            response_format,
            speed,
            instructions: text(arguments, "instructions")?,
            extra_headers: IndexMap::new(),
        },
        timeout: speech_timeout(arguments)?,
        max_retries: max_retries(arguments)?,
    })
}

pub(crate) fn speech_timeout(arguments: &RequestMap) -> Result<Option<Duration>, ConvertError> {
    match arguments.get("timeout") {
        Some(Json::Number(value)) => {
            let seconds = value.as_f64().ok_or_else(|| field("timeout", "a number"))?;
            Duration::try_from_secs_f64(seconds)
                .map(Some)
                .map_err(|source| ConvertError::Timeout { source })
        }
        None | Some(Json::Null) => Ok(None),
        Some(_other) => Err(field("timeout", "a number")),
    }
}

fn speech_format(name: Option<&str>) -> Result<Option<SpeechFormat>, ConvertError> {
    match name {
        None => Ok(None),
        Some("mp3") => Ok(Some(SpeechFormat::Mp3)),
        Some("opus") => Ok(Some(SpeechFormat::Opus)),
        Some("aac") => Ok(Some(SpeechFormat::Aac)),
        Some("flac") => Ok(Some(SpeechFormat::Flac)),
        Some("wav") => Ok(Some(SpeechFormat::Wav)),
        Some("pcm") => Ok(Some(SpeechFormat::Pcm)),
        Some(_other) => Err(field("response_format", "mp3, opus, aac, flac, wav or pcm")),
    }
}
