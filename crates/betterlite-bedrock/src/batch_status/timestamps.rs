use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};

// int(timestamp) truncates toward zero, so a pre-1970 moment with a fraction moves up a second
pub(crate) const fn epoch_seconds(moment: DateTime<Utc>) -> i64 {
    let seconds = moment.timestamp();
    if seconds < 0 && moment.timestamp_subsec_nanos() > 0 {
        return seconds.saturating_add(1);
    }
    seconds
}

// LiteLLM feeds datetime.fromisoformat the str() of the value with every Z spelled as +00:00
#[must_use]
pub fn parse_iso_timestamp(text: &str) -> Option<i64> {
    if text.is_empty() {
        return None;
    }
    let normalized = text.replace('Z', "+00:00").replace(',', ".");
    if let Ok(moment) = DateTime::parse_from_rfc3339(&normalized) {
        return Some(epoch_seconds(moment.with_timezone(&Utc)));
    }
    let naive = NaiveDateTime::parse_from_str(&normalized, "%Y-%m-%dT%H:%M:%S%.f")
        .ok()
        .or_else(|| NaiveDateTime::parse_from_str(&normalized, "%Y-%m-%d %H:%M:%S%.f").ok())
        .or_else(|| NaiveDateTime::parse_from_str(&normalized, "%Y-%m-%dT%H:%M").ok())
        .or_else(|| NaiveDateTime::parse_from_str(&normalized, "%Y-%m-%d %H:%M").ok())
        .or_else(|| {
            let date = NaiveDate::parse_from_str(&normalized, "%Y-%m-%d").ok()?;
            date.and_hms_opt(0, 0, 0)
        })?;
    Some(epoch_seconds(naive.and_utc()))
}
