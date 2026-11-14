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
}

#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptionWordTiming {
    pub word: String,
    pub start: Option<f64>,
    pub end: Option<f64>,
    pub speaker: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Seconds {
    Number(f64),
    Flag(bool),
    Text(String),
}

impl Seconds {
    fn value(&self) -> Result<f64, SubtitleTimingError> {
        match self {
            Self::Number(number) => Ok(*number),
            Self::Flag(flag) => Ok(if *flag { 1.0 } else { 0.0 }),
            Self::Text(text) => text
                .trim()
                .parse::<f64>()
                .map_err(|_| SubtitleTimingError::NotANumber),
        }
    }
}

#[derive(Debug, Deserialize)]
struct WordTimingRecord {
    #[serde(default)]
    word: String,
    #[serde(default)]
    start: Option<Seconds>,
    #[serde(default)]
    end: Option<Seconds>,
    #[serde(default)]
    speaker: Option<String>,
}

impl WordTimingRecord {
    fn timing(&self) -> Result<TranscriptionWordTiming, SubtitleTimingError> {
        Ok(TranscriptionWordTiming {
            word: self.word.clone(),
            start: self.start.as_ref().map(Seconds::value).transpose()?,
            end: self.end.as_ref().map(Seconds::value).transpose()?,
            speaker: self.speaker.clone(),
        })
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
    let Ok(records) = serde_json::from_value::<Vec<WordTimingRecord>>(words.clone()) else {
        return Vec::new();
    };
    records
        .iter()
        .map(WordTimingRecord::timing)
        .collect::<Result<Vec<_>, _>>()
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
