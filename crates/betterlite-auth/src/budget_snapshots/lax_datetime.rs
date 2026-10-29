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
#[error("{0}")]
pub struct LaxDateTimeConfigError(speedate::ConfigError);

#[derive(Debug, Error)]
#[error("{0}")]
pub struct LaxDateTimeParseError(speedate::ParseError);

#[derive(Debug, Error)]
pub enum LaxDateTimeError {
    #[error("invalid microseconds precision overflow configuration")]
    Config {
        #[source]
        source: LaxDateTimeConfigError,
    },
    #[error("invalid datetime: {value}")]
    Parse {
        value: String,
        #[source]
        source: LaxDateTimeParseError,
    },
}

impl TryFrom<LaxDateTimeRaw> for LaxDateTime {
    type Error = LaxDateTimeError;

    fn try_from(raw: LaxDateTimeRaw) -> Result<Self, Self::Error> {
        let text = match raw {
            LaxDateTimeRaw::Text(text) => text,
            LaxDateTimeRaw::Number(number) => number.to_string(),
        };
        let config = build_config().map_err(|source| LaxDateTimeError::Config {
            source: LaxDateTimeConfigError(source),
        })?;
        parse_lax_datetime(&text, &config).map_err(|source| LaxDateTimeError::Parse {
            value: text,
            source: LaxDateTimeParseError(source),
        })
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

fn parse_lax_datetime(
    text: &str,
    config: &DateTimeConfig,
) -> Result<LaxDateTime, speedate::ParseError> {
    let parsed = DateTime::parse_str_with_config(text, config).or_else(|error| {
        Date::parse_str(text)
            .and_then(|date| DateTime::parse_str_with_config(&format!("{date}T00:00:00"), config))
            .map_err(|_date_error| error)
    })?;
    let formatted = parsed.to_string();
    Ok(if parsed.time.tz_offset.is_some() {
        LaxDateTime::Offset(formatted)
    } else {
        LaxDateTime::Naive(formatted)
    })
}
