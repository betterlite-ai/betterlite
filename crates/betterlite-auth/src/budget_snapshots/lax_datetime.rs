mod tests;

use serde::{Deserialize, Serialize};
use serde_json::Number;
use speedate::{Date, DateTime, DateTimeConfig, TimeConfig, TimestampUnit};
use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged, try_from = "LaxDateTimeRaw")]
pub enum LaxDateTime {
    Offset(String),
    Naive(String),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum LaxDateTimeRaw {
    Text(String),
    Number(Number),
}

#[derive(Debug, Error)]
#[error("invalid datetime: {value}")]
pub struct LaxDateTimeError {
    value: String,
}

impl TryFrom<LaxDateTimeRaw> for LaxDateTime {
    type Error = LaxDateTimeError;

    fn try_from(raw: LaxDateTimeRaw) -> Result<Self, Self::Error> {
        let text = match raw {
            LaxDateTimeRaw::Text(text) => text,
            LaxDateTimeRaw::Number(number) => number.to_string(),
        };
        parse_lax_datetime(&text).ok_or(LaxDateTimeError { value: text })
    }
}

fn build_config() -> Result<DateTimeConfig, speedate::ConfigError> {
    let time_config = TimeConfig::builder()
        .unix_timestamp_offset(Some(0_i32))
        .microseconds_precision_overflow_behavior("truncate".parse()?)
        .build();
    Ok(DateTimeConfig::builder()
        .timestamp_unit(TimestampUnit::Infer)
        .time_config(time_config)
        .build())
}

fn parse_lax_datetime(text: &str) -> Option<LaxDateTime> {
    let config = build_config().ok()?;
    let parsed = DateTime::parse_str_with_config(text, &config)
        .or_else(|_| {
            let date = Date::parse_str(text)?;
            DateTime::parse_str_with_config(&format!("{date}T00:00:00"), &config)
        })
        .ok()?;
    let formatted = parsed.to_string();
    Some(if parsed.time.tz_offset.is_some() {
        LaxDateTime::Offset(formatted)
    } else {
        LaxDateTime::Naive(formatted)
    })
}
