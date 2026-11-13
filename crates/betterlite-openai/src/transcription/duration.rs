use std::str::FromStr as _;

use betterlite_models::decisions::Json;
use fancy_regex::Regex;
use rust_decimal::Decimal;
use serde_json::Number;

use super::error::TranscriptionError;

// srt_vtt_duration: the largest hh:mm:ss,mmm or hh:mm:ss.mmm stamp, in exact seconds.
#[must_use]
pub fn srt_vtt_duration(content: &str) -> Option<Decimal> {
    let pattern = Regex::new(r"(\d{2}):(\d{2}):(\d{2})[.,](\d{3})").ok()?;
    pattern
        .captures_iter(content)
        .flatten()
        .filter_map(|found| {
            let part = |index: usize| found.get(index)?.as_str().parse::<u64>().ok();
            let seconds = [part(1)?, part(2)?, part(3)?]
                .into_iter()
                .try_fold(0_u64, |total, unit| {
                    total.checked_mul(60)?.checked_add(unit)
                })?;
            let millis = seconds.checked_mul(1000)?.checked_add(part(4)?)?;
            Some(Decimal::new(i64::try_from(millis).ok()?, 3))
        })
        .max()
}

// LiteLLM hands this value to Python as a float, so an integral duration keeps a fractional part.
pub fn duration_number(duration: Option<Decimal>) -> Result<Json, TranscriptionError> {
    let Some(found) = duration else {
        return Ok(Json::Null);
    };
    let text = found.normalize().to_string();
    let rendered = if text.contains('.') {
        text
    } else {
        format!("{text}.0")
    };
    Number::from_str(&rendered)
        .map(Json::Number)
        .map_err(|source| TranscriptionError::Json { source })
}
