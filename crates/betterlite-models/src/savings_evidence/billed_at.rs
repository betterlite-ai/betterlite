use std::borrow::Cow;

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};

pub enum BilledAt {
    Missing,
    Timestamp(DateTime<Utc>),
    Text(String),
}

// Mirrors fromisoformat(value.replace("Z", "+00:00")): a naive datetime or a bare date is read as UTC.
#[must_use]
pub fn decode_billed_at(text: &str) -> Option<DateTime<Utc>> {
    let normalized = text.replace('Z', "+00:00");
    if let Some(parsed) = decode_dashed_date_and_time(&normalized) {
        return Some(parsed);
    }
    for format in ["%Y%m%dT%H%M%S%.f%z", "%Y%m%dT%H%M%S%z"] {
        if let Ok(parsed) = DateTime::parse_from_str(&normalized, format) {
            return Some(parsed.with_timezone(&Utc));
        }
    }
    for format in ["%Y%m%dT%H%M%S%.f", "%Y%m%dT%H%M%S"] {
        if let Ok(parsed) = NaiveDateTime::parse_from_str(&normalized, format) {
            return Some(parsed.and_utc());
        }
    }
    for format in ["%Y-%m-%d", "%Y%m%d"] {
        if let Ok(parsed) = NaiveDate::parse_from_str(&normalized, format) {
            return Some(parsed.and_time(NaiveTime::MIN).and_utc());
        }
    }
    None
}

// fromisoformat allows any single separator character and a time that stops at the hour or minute.
fn decode_dashed_date_and_time(normalized: &str) -> Option<DateTime<Utc>> {
    let date_part = normalized.get(..10)?;
    let rest = normalized.get(10..)?;
    let separator = rest.chars().next()?;
    let time_part = pad_bare_hour(rest.get(separator.len_utf8()..)?);
    let rebuilt = format!("{date_part}T{time_part}");
    for format in [
        "%Y-%m-%dT%H:%M:%S%.f%z",
        "%Y-%m-%dT%H:%M:%S%z",
        "%Y-%m-%dT%H:%M%z",
    ] {
        if let Ok(parsed) = DateTime::parse_from_str(&rebuilt, format) {
            return Some(parsed.with_timezone(&Utc));
        }
    }
    for format in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
    ] {
        if let Ok(parsed) = NaiveDateTime::parse_from_str(&rebuilt, format) {
            return Some(parsed.and_utc());
        }
    }
    None
}

// chrono has no bare "%H" format, so a bare hour gets a ":00" minute inserted ahead of any offset.
fn pad_bare_hour(time_part: &str) -> Cow<'_, str> {
    let (Some(hour), Some(after_hour)) = (time_part.get(..2), time_part.get(2..)) else {
        return Cow::Borrowed(time_part);
    };
    if hour.chars().all(|digit| digit.is_ascii_digit()) && !after_hour.starts_with(':') {
        Cow::Owned(format!("{hour}:00{after_hour}"))
    } else {
        Cow::Borrowed(time_part)
    }
}

#[must_use]
pub fn coerce_billed_at(value: BilledAt) -> Option<DateTime<Utc>> {
    match value {
        BilledAt::Missing => None,
        BilledAt::Timestamp(timestamp) => Some(timestamp),
        BilledAt::Text(text) => decode_billed_at(&text),
    }
}

mod tests;
