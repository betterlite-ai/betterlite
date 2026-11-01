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
    for format in [
        "%Y-%m-%dT%H:%M:%S%.f%z",
        "%Y-%m-%dT%H:%M:%S%z",
        "%Y%m%dT%H%M%S%.f%z",
        "%Y%m%dT%H%M%S%z",
    ] {
        if let Ok(parsed) = DateTime::parse_from_str(&normalized, format) {
            return Some(parsed.with_timezone(&Utc));
        }
    }
    for format in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y%m%dT%H%M%S%.f",
        "%Y%m%dT%H%M%S",
    ] {
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

#[must_use]
pub fn coerce_billed_at(value: BilledAt) -> Option<DateTime<Utc>> {
    match value {
        BilledAt::Missing => None,
        BilledAt::Timestamp(timestamp) => Some(timestamp),
        BilledAt::Text(text) => decode_billed_at(&text),
    }
}

mod tests;
