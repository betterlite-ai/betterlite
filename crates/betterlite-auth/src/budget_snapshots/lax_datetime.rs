mod tests;

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime};
use serde::{Deserialize, Serialize};

// RFC 3339, a bare date, a space-separated naive string, or a Unix-second timestamp.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
#[serde(try_from = "LaxDateTimeRaw")]
pub enum LaxDateTime {
    Offset(DateTime<FixedOffset>),
    Naive(NaiveDateTime),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum LaxDateTimeRaw {
    Str(String),
    I64(i64),
    U64(u64),
}

impl TryFrom<LaxDateTimeRaw> for LaxDateTime {
    type Error = String;

    fn try_from(raw: LaxDateTimeRaw) -> Result<Self, Self::Error> {
        match raw {
            LaxDateTimeRaw::Str(value) => {
                parse_lax_datetime(&value).ok_or_else(|| format!("invalid datetime: {value}"))
            }
            LaxDateTimeRaw::I64(seconds) => {
                timestamp_to_utc(seconds).ok_or_else(|| "timestamp out of range".to_owned())
            }
            LaxDateTimeRaw::U64(seconds) => i64::try_from(seconds)
                .ok()
                .and_then(timestamp_to_utc)
                .ok_or_else(|| "timestamp out of range".to_owned()),
        }
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
