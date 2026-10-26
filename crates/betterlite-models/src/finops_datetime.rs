use chrono::{DateTime, TimeZone as _, Utc};
use serde::de::Error;
use serde::{Deserialize, Deserializer};
use serde_json::Number;
use speedate::{Date, DateTime as SpeedateDateTime};

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
    let parsed = SpeedateDateTime::parse_str(&text).or_else(|_| {
        let date = Date::parse_str(&text)?;
        SpeedateDateTime::parse_str(&format!("{date}T00:00:00"))
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
