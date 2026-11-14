use std::num::ParseFloatError;

use num_traits::ToPrimitive as _;
use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;

use super::grouping::group_tokens_into_cues;
use super::render::{render_subtitle_tokens_as_srt, render_subtitle_tokens_as_vtt};
use super::{SRT_RESPONSE_FORMAT, SUBTITLE_RESPONSE_FORMATS, Speaker, SubtitleToken};

#[derive(Debug, PartialEq, Eq, Error)]
pub enum SubtitleTimingError {
    #[error("a word timing is not a number")]
    NotANumber,
    #[error("a word timing rounds outside the millisecond range")]
    OutOfRange,
    #[error("a word timing string is not a number: {0}")]
    Unparsable(#[from] ParseFloatError),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptionWordTiming {
    pub word: String,
    pub start: Option<f64>,
    pub end: Option<f64>,
    pub speaker: Option<String>,
}

#[derive(Deserialize)]
struct RawWordTiming {
    #[serde(default)]
    word: String,
    #[serde(default)]
    start: Option<Value>,
    #[serde(default)]
    end: Option<Value>,
    #[serde(default)]
    speaker: Option<String>,
}

impl TryFrom<RawWordTiming> for TranscriptionWordTiming {
    type Error = SubtitleTimingError;

    fn try_from(raw: RawWordTiming) -> Result<Self, Self::Error> {
        Ok(Self {
            word: raw.word,
            start: lax_seconds(raw.start)?,
            end: lax_seconds(raw.end)?,
            speaker: raw.speaker,
        })
    }
}

fn lax_seconds(value: Option<Value>) -> Result<Option<f64>, SubtitleTimingError> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(number)) => number
            .as_f64()
            .map(Some)
            .ok_or(SubtitleTimingError::OutOfRange),
        Some(Value::Bool(flag)) => Ok(Some(f64::from(flag))),
        Some(Value::String(text)) => Ok(Some(text.trim().parse::<f64>()?)),
        Some(Value::Array(_) | Value::Object(_)) => Err(SubtitleTimingError::NotANumber),
    }
}

pub fn seconds_to_ms(maybe_seconds: Option<f64>) -> Result<Option<i64>, SubtitleTimingError> {
    let Some(seconds) = maybe_seconds else {
        return Ok(None);
    };
    if seconds.is_nan() {
        return Err(SubtitleTimingError::NotANumber);
    }
    // float_arithmetic is denied; a fused multiply-add with zero is the same product
    let millis = seconds.mul_add(1000.0, 0.0);
    if !millis.is_finite() {
        return Err(SubtitleTimingError::OutOfRange);
    }
    millis
        .round_ties_even()
        .to_i64()
        .map(Some)
        .ok_or(SubtitleTimingError::OutOfRange)
}

pub fn word_to_subtitle_token(
    word: &TranscriptionWordTiming,
) -> Result<SubtitleToken, SubtitleTimingError> {
    Ok(SubtitleToken {
        text: format!("{} ", word.word),
        start_ms: seconds_to_ms(word.start)?,
        end_ms: seconds_to_ms(word.end)?,
        speaker: word.speaker.clone().map(Speaker::Text),
    })
}

#[must_use]
pub fn parse_word_timings(words: &Value) -> Vec<TranscriptionWordTiming> {
    let Ok(records) = serde_json::from_value::<Vec<RawWordTiming>>(words.clone()) else {
        return Vec::new();
    };
    records
        .into_iter()
        .map(TranscriptionWordTiming::try_from)
        .collect::<Result<_, _>>()
        .unwrap_or_default()
}

pub fn synthesize_subtitle_document(
    words: &Value,
    response_format: &str,
) -> Result<Option<String>, SubtitleTimingError> {
    if !SUBTITLE_RESPONSE_FORMATS.contains(&response_format) {
        return Ok(None);
    }
    let tokens = parse_word_timings(words)
        .iter()
        .map(word_to_subtitle_token)
        .collect::<Result<Vec<_>, _>>()?;
    let cues = group_tokens_into_cues(&tokens);
    if cues.is_empty() {
        return Ok(None);
    }
    if response_format == SRT_RESPONSE_FORMAT {
        return Ok(Some(render_subtitle_tokens_as_srt(&tokens)));
    }
    Ok(Some(render_subtitle_tokens_as_vtt(&tokens)))
}
