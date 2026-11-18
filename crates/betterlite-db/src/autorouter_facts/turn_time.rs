use chrono::{DateTime, NaiveDate, NaiveDateTime};

const NAIVE_TURN_TIME_FORMATS: &[&str] = &[
    "%Y-%m-%dT%H:%M:%S%.f",
    "%Y-%m-%d %H:%M:%S%.f",
    "%Y-%m-%dT%H:%M",
    "%Y-%m-%d %H:%M",
];

const OFFSET_TURN_TIME_FORMATS: &[&str] = &["%Y-%m-%dT%H:%M:%S%.f%z", "%Y-%m-%d %H:%M:%S%.f%z"];

// _turn_time_utc: a naive time stays naive, and an aware one converts to UTC and drops the offset.
#[must_use]
pub fn turn_time_utc(start_time_iso: &str) -> Option<NaiveDateTime> {
    if let Ok(time) = DateTime::parse_from_rfc3339(start_time_iso) {
        return Some(time.naive_utc());
    }
    for format in OFFSET_TURN_TIME_FORMATS {
        if let Ok(time) = DateTime::parse_from_str(start_time_iso, format) {
            return Some(time.naive_utc());
        }
    }
    for format in NAIVE_TURN_TIME_FORMATS {
        if let Ok(time) = NaiveDateTime::parse_from_str(start_time_iso, format) {
            return Some(time);
        }
    }
    NaiveDate::parse_from_str(start_time_iso, "%Y-%m-%d")
        .ok()
        .and_then(|date| date.and_hms_opt(0, 0, 0))
}

// str(payload.get("startTime") or "") is what the source parses, and bytes that are not UTF-8 are not a timestamp.
#[must_use]
pub fn decode_turn_time(bytes: &[u8]) -> Option<NaiveDateTime> {
    let text = std::str::from_utf8(bytes).ok()?;
    turn_time_utc(text)
}

#[path = "turn_time/tests.rs"]
mod tests;
