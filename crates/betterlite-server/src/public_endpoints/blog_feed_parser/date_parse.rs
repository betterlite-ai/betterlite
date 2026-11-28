use betterlite_core::email_date::{ParsedDate, parsedate_tz};
use chrono::{NaiveDate, NaiveTime};

mod tests;

// datetime.timezone refuses an offset of 24 hours in either direction.
const MAX_ZONE_SECONDS: u64 = 86_400;

// Ports `email.utils.parsedate_to_datetime`, whose `datetime` and `timezone` constructors reject these fields.
pub(super) fn parse_date(raw: &str) -> String {
    let Some(parsed) = parsedate_tz(raw) else {
        return raw.to_owned();
    };
    let zone_too_wide = parsed
        .zone_offset
        .is_some_and(|offset| offset.unsigned_abs() >= MAX_ZONE_SECONDS);
    if zone_too_wide {
        return raw.to_owned();
    }
    formatted(&parsed).unwrap_or_else(|| raw.to_owned())
}

fn formatted(parsed: &ParsedDate) -> Option<String> {
    let year = i32::try_from(parsed.year).ok()?;
    if !(1_i32..=9_999_i32).contains(&year) {
        return None;
    }
    let day = u32::try_from(parsed.day).ok()?;
    let hour = u32::try_from(parsed.hour).ok()?;
    let minute = u32::try_from(parsed.minute).ok()?;
    let second = u32::try_from(parsed.second).ok()?;
    NaiveTime::from_hms_opt(hour, minute, second)?;
    let date = NaiveDate::from_ymd_opt(year, parsed.month, day)?;
    Some(date.format("%Y-%m-%d").to_string())
}
