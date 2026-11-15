use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, ParseError};

pub(super) fn parse_created_since(text: &str) -> Result<NaiveDateTime, ParseError> {
    if text.contains('_') {
        return NaiveDateTime::parse_from_str(text, "%Y-%m-%d_%H:%M");
    }
    let date = NaiveDate::parse_from_str(text, "%Y-%m-%d")?;
    Ok(date.and_time(NaiveTime::MIN))
}

pub(super) fn parse_created_at(text: &str) -> Result<NaiveDateTime, ParseError> {
    // LiteLLM drops tzinfo instead of converting, so the wall time as written is what the filter compares.
    if let Ok(parsed) = DateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f%#z") {
        return Ok(parsed.naive_local());
    }
    NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f")
}
