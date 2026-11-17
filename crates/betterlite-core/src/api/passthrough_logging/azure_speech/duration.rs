use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_file_input::decoder::{AudioDecodeLimits, decode_audio_duration};
use indexmap::IndexMap;
use rust_decimal::Decimal;
use serde_json::Number;

use super::route::SpeechRoute;
use super::{AzureSpeechLoggingError, MILLISECONDS_PER_SECOND, TICKS_PER_SECOND};

// The short route bills the longer of the uploaded audio and the recognition result; a batch route bills nothing.
pub fn billed_audio_seconds(
    route: SpeechRoute,
    request_body: Option<&[u8]>,
    response_body: &[u8],
) -> Result<Decimal, AzureSpeechLoggingError> {
    match route {
        SpeechRoute::ShortAudio => {
            let uploaded = request_body
                .and_then(|audio| decode_audio_duration(audio, &AudioDecodeLimits::default()))
                .unwrap_or(Decimal::ZERO);
            Ok(uploaded.max(reported_audio_seconds(route, response_body)?))
        }
        SpeechRoute::FastTranscription | SpeechRoute::Batch => {
            reported_audio_seconds(route, response_body)
        }
    }
}

// Python's isinstance(value, int) reads a bool as 0 or 1 and refuses a float or a string.
fn json_integer(
    value: Option<&Json>,
    field: &'static str,
) -> Result<Option<Decimal>, AzureSpeechLoggingError> {
    match value {
        Some(Json::Bool(flag)) => Ok(Some(Decimal::from(u8::from(*flag)))),
        Some(Json::Number(number)) => integer_literal(number, field),
        Some(Json::Null | Json::Text(_) | Json::List(_) | Json::Object(_)) | None => Ok(None),
    }
}

fn integer_literal(
    number: &Number,
    field: &'static str,
) -> Result<Option<Decimal>, AzureSpeechLoggingError> {
    let text = number.to_string();
    if text.contains(['.', 'e', 'E']) {
        return Ok(None);
    }
    match text.parse::<Decimal>() {
        Ok(parsed) => Ok(Some(parsed)),
        Err(_source) => Err(AzureSpeechLoggingError::FieldValue { field, value: text }),
    }
}

// A provider reply is a JSON object; a body of another shape reports no duration.
fn reply_fields(response_body: &[u8]) -> Option<IndexMap<String, Json>> {
    let Ok(Json::Object(fields)) = serde_json::from_slice::<Json>(response_body) else {
        return None;
    };
    Some(fields)
}

// Offset and Duration are 100-nanosecond ticks from the start of the audio, and the fast route reports milliseconds.
fn reported_audio_seconds(
    route: SpeechRoute,
    response_body: &[u8],
) -> Result<Decimal, AzureSpeechLoggingError> {
    match route {
        SpeechRoute::ShortAudio => {
            let Some(fields) = reply_fields(response_body) else {
                return Ok(Decimal::ZERO);
            };
            let (Some(offset), Some(duration)) = (
                json_integer(fields.get("Offset"), "Offset")?,
                json_integer(fields.get("Duration"), "Duration")?,
            ) else {
                return Ok(Decimal::ZERO);
            };
            let ticks = offset
                .checked_add(duration)
                .ok_or(AzureSpeechLoggingError::DurationRange)?;
            ticks
                .checked_div(Decimal::from(TICKS_PER_SECOND))
                .ok_or(AzureSpeechLoggingError::DurationRange)
        }
        SpeechRoute::FastTranscription => {
            let Some(fields) = reply_fields(response_body) else {
                return Ok(Decimal::ZERO);
            };
            let Some(milliseconds) =
                json_integer(fields.get("durationMilliseconds"), "durationMilliseconds")?
            else {
                return Ok(Decimal::ZERO);
            };
            milliseconds
                .checked_div(Decimal::from(MILLISECONDS_PER_SECOND))
                .ok_or(AzureSpeechLoggingError::DurationRange)
        }
        SpeechRoute::Batch => Ok(Decimal::ZERO),
    }
}
