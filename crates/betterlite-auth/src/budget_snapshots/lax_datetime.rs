mod tests;

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime};
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;

/// `LiteLLM`'s lax `datetime | None` fields: an RFC 3339 string, a bare date, a
/// space-separated naive string, or a Unix-second timestamp.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum LaxDateTime {
    Offset(DateTime<FixedOffset>),
    Naive(NaiveDateTime),
}

impl<'de> Deserialize<'de> for LaxDateTime {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(LaxDateTimeVisitor)
    }
}

struct LaxDateTimeVisitor;

impl Visitor<'_> for LaxDateTimeVisitor {
    type Value = LaxDateTime;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an RFC 3339 datetime, a date, or a Unix timestamp")
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        parse_lax_datetime(v).ok_or_else(|| E::custom("invalid datetime"))
    }

    fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        timestamp_to_utc(v).ok_or_else(|| E::custom("timestamp out of range"))
    }

    fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        i64::try_from(v)
            .ok()
            .and_then(timestamp_to_utc)
            .ok_or_else(|| E::custom("timestamp out of range"))
    }
}

fn timestamp_to_utc(seconds: i64) -> Option<LaxDateTime> {
    let utc = DateTime::from_timestamp(seconds, 0)?;
    let offset = FixedOffset::east_opt(0)?;
    Some(LaxDateTime::Offset(utc.with_timezone(&offset)))
}

fn parse_lax_datetime(value: &str) -> Option<LaxDateTime> {
    if let Ok(offset) = DateTime::parse_from_rfc3339(value) {
        return Some(LaxDateTime::Offset(offset));
    }
    for format in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(value, format) {
            return Some(LaxDateTime::Naive(naive));
        }
    }
    let date = NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()?;
    date.and_hms_opt(0, 0, 0).map(LaxDateTime::Naive)
}
