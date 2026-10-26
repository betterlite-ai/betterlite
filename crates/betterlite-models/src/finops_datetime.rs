use chrono::{DateTime, TimeZone as _, Utc};
use serde::de::Error;
use serde::{Deserialize, Deserializer};
use serde_json::Number;
use speedate::{Date, DateTime as SpeedateDateTime, DateTimeConfig, TimeConfig, TimestampUnit};

#[derive(Deserialize)]
#[serde(untagged)]
enum DateTimeInput {
    Text(String),
    Number(Number),
}

pub(crate) fn deserialize_optional_datetime<'de, D>(
    deserializer: D,
) -> Result<Option<DateTime<Utc>>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<DateTimeInput>::deserialize(deserializer)?;
    value
        .map(|input| parse_datetime(input).map_err(Error::custom))
        .transpose()
}

fn parse_datetime(input: DateTimeInput) -> Result<DateTime<Utc>, speedate::ParseError> {
    let text = match input {
        DateTimeInput::Text(text) => text,
        DateTimeInput::Number(number) => number.to_string(),
    };
    let config = DateTimeConfig {
        timestamp_unit: TimestampUnit::Infer,
        time_config: TimeConfig::default(),
    };
    let parsed = SpeedateDateTime::parse_str_with_config(&text, &config).or_else(|_| {
        let date = Date::parse_str(&text)?;
        SpeedateDateTime::parse_str_with_config(&format!("{date}T00:00:00"), &config)
    })?;
    let nanoseconds = parsed
        .time
        .microsecond
        .checked_mul(1_000)
        .ok_or(speedate::ParseError::TimeTooLarge)?;
    Utc.timestamp_opt(parsed.timestamp_tz(), nanoseconds)
        .single()
        .ok_or(speedate::ParseError::TimeTooLarge)
}
