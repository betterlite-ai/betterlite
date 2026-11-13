use serde::{Deserialize, Serialize};
use speedate::{Date, DateTime, DateTimeConfig, TimeConfig, TimestampUnit};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Input")]
pub struct Timestamp(String);

#[derive(Deserialize)]
#[serde(untagged)]
enum Input {
    Text(String),
    Number(serde_json::Number),
}

#[derive(Debug, Error)]
pub enum TimestampError {
    // speedate's diagnostic enums do not implement std::error::Error.
    #[error("invalid timestamp precision: {0}")]
    Precision(speedate::ConfigError),
    #[error("invalid timestamp: {0}")]
    Invalid(speedate::ParseError),
}

impl TryFrom<Input> for Timestamp {
    type Error = TimestampError;

    fn try_from(input: Input) -> Result<Self, Self::Error> {
        let text = match input {
            Input::Text(text) => text,
            Input::Number(number) => number.to_string(),
        };
        let config = DateTimeConfig {
            timestamp_unit: TimestampUnit::Infer,
            time_config: TimeConfig::builder()
                .microseconds_precision_overflow_behavior(
                    "truncate".parse().map_err(TimestampError::Precision)?,
                )
                .unix_timestamp_offset(Some(0_i32))
                .build(),
        };
        DateTime::parse_str_with_config(&text, &config).map_or_else(
            |_| {
                Date::parse_str(&text)
                    .map(|date| Self(format!("{date}T00:00:00")))
                    .map_err(TimestampError::Invalid)
            },
            |timestamp| Ok(Self(timestamp.to_string())),
        )
    }
}
