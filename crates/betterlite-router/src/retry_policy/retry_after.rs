use std::mem;
use std::str::{Utf8Error, from_utf8};

use chrono::{Datelike as _, NaiveDate};
use thiserror::Error;

// the parser never allocates past this many input bytes, so a header from a provider cannot size a request
pub const MAX_RETRY_AFTER_BYTES: usize = 8192;
const MONTHS: [(&str, &str); 12] = [
    ("jan", "january"),
    ("feb", "february"),
    ("mar", "march"),
    ("apr", "april"),
    ("may", "may"),
    ("jun", "june"),
    ("jul", "july"),
    ("aug", "august"),
    ("sep", "september"),
    ("oct", "october"),
    ("nov", "november"),
    ("dec", "december"),
];
const WEEKDAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
// email._parseaddr._timezones, in hours and minutes as the header writes them
const ZONES: [(&str, i64); 14] = [
    ("UT", 0),
    ("UTC", 0),
    ("GMT", 0),
    ("Z", 0),
    ("AST", -400),
    ("ADT", -300),
    ("EST", -500),
    ("EDT", -400),
    ("CST", -600),
    ("CDT", -500),
    ("MST", -700),
    ("MDT", -600),
    ("PST", -800),
    ("PDT", -700),
];
// Python's date(1970, 1, 1).toordinal(), which counts days the way chrono's num_days_from_ce does
const EPOCH_ORDINAL: i64 = 719_163;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryAfter {
    Seconds(i64),
    At { unix_millis: i64 },
}

impl RetryAfter {
    // Python's int(retry_date - time.time()) truncates toward zero, which Rust's checked division does too
    #[must_use]
    pub fn seconds_until(self, now_unix_millis: i64) -> i64 {
        match self {
            Self::Seconds(seconds) => seconds,
            Self::At { unix_millis } => unix_millis
                .saturating_sub(now_unix_millis)
                .checked_div(1000)
                .unwrap_or(0),
        }
    }
}

#[derive(Debug, Error)]
pub enum RetryAfterError {
    #[error("the retry-after value is {length} bytes, over the 8192-byte limit")]
    TooLarge { length: usize },
    #[error("the retry-after value is not UTF-8")]
    NotUtf8 {
        #[source]
        source: Utf8Error,
    },
    #[error("the retry-after value is neither whole seconds nor a date")]
    NotDate,
}

pub fn parse_retry_after(bytes: &[u8]) -> Result<RetryAfter, RetryAfterError> {
    if bytes.len() > MAX_RETRY_AFTER_BYTES {
        return Err(RetryAfterError::TooLarge {
            length: bytes.len(),
        });
    }
    let text = from_utf8(bytes).map_err(|source| RetryAfterError::NotUtf8 { source })?;
    let header = text.trim();
    if let Some(seconds) = parse_integer(header) {
        return Ok(RetryAfter::Seconds(seconds));
    }
    let unix_seconds = parse_date(header).ok_or(RetryAfterError::NotDate)?;
    Ok(RetryAfter::At {
        unix_millis: unix_seconds.saturating_mul(1000),
    })
}

// email.utils.parsedate_tz splits on whitespace, drops a leading weekday unchecked and unpacks the RFC 850 form
fn date_tokens(header: &str) -> Option<Vec<&str>> {
    let mut words = header.split_whitespace();
    let first = words.next()?;
    let mut tokens: Vec<&str> = Vec::new();
    if !first.ends_with(',') && !WEEKDAYS.contains(&first.to_lowercase().as_str()) {
        tokens.push(first.rsplit_once(',').map_or(first, |(_, rest)| rest));
    }
    tokens.extend(words);
    if let [dated, second, third] = *tokens.as_slice() {
        let parts: Vec<&str> = dated.split('-').collect();
        if parts.len() == 3 {
            tokens = parts;
            tokens.extend([second, third]);
        }
    }
    if let [day, month, year, last] = *tokens.as_slice() {
        let split = last
            .find('+')
            .or_else(|| last.find('-'))
            .filter(|index| *index > 0)
            .and_then(|index| last.split_at_checked(index));
        let (time, zone) = split.unwrap_or((last, ""));
        tokens = vec![day, month, year, time, zone];
    }
    (tokens.len() >= 5).then_some(tokens)
}

// the fields may come in either order, as parsedate_tz swaps day and month, year and time, year and zone
fn parse_date(header: &str) -> Option<i64> {
    let tokens = date_tokens(header)?;
    let [mut day, month, mut year, mut time, mut zone, ..] = *tokens.as_slice() else {
        return None;
    };
    let mut month_name = month.to_lowercase();
    if !MONTHS
        .iter()
        .any(|names| names.0 == month_name || names.1 == month_name)
    {
        month_name = day.to_lowercase();
        day = month;
    }
    let month_index = MONTHS
        .iter()
        .position(|names| names.0 == month_name || names.1 == month_name)?;
    day = day.strip_suffix(',').unwrap_or(day);
    if year.find(':').is_some_and(|index| index > 0) {
        mem::swap(&mut year, &mut time);
    }
    year = year.strip_suffix(',').unwrap_or(year);
    if year.is_empty() {
        return None;
    }
    if !year.starts_with(|first: char| first.is_ascii_digit()) {
        mem::swap(&mut year, &mut zone);
    }
    time = time.strip_suffix(',').unwrap_or(time);
    unix_seconds(year, month_index, day, time, zone)
}

// calendar.timegm leaves day, hour, minute and second unchecked, so 31 Feb or 25:00 roll over as plain arithmetic
fn unix_seconds(year: &str, month_index: usize, day: &str, time: &str, zone: &str) -> Option<i64> {
    // parsedate_tz falls back to dots only when the time has no colon at all
    let separator = if time.contains(':') { ':' } else { '.' };
    let clock: Vec<&str> = time.split(separator).collect();
    let (hour, minute, second) = match *clock.as_slice() {
        [hour_text, minute_text] => (hour_text, minute_text, "0"),
        [hour_text, minute_text, second_text] => (hour_text, minute_text, second_text),
        _ => return None,
    };
    let mut full_year = parse_integer(year)?;
    if full_year < 100 {
        full_year = full_year.saturating_add(if full_year > 68 { 1900 } else { 2000 });
    }
    // datetime.date raises outside years 1 to 9999, and LiteLLM then ignores the header
    let calendar_year = i32::try_from(full_year)
        .ok()
        .filter(|value| (1_i32..=9_999_i32).contains(value))?;
    let month = u32::try_from(month_index).ok()?.checked_add(1)?;
    let first_of_month = NaiveDate::from_ymd_opt(calendar_year, month, 1)?;
    let days = i64::from(first_of_month.num_days_from_ce())
        .checked_sub(EPOCH_ORDINAL)?
        .checked_add(parse_integer(day)?)?
        .checked_sub(1)?;
    let hours = days.checked_mul(24)?.checked_add(parse_integer(hour)?)?;
    let minutes = hours.checked_mul(60)?.checked_add(parse_integer(minute)?)?;
    let seconds = minutes
        .checked_mul(60)?
        .checked_add(parse_integer(second)?)?;
    seconds.checked_sub(zone_offset(zone)?)
}

// a zone that is neither a known name nor a number counts as UTC, since parsedate_tz turns its None into 0
fn zone_offset(zone: &str) -> Option<i64> {
    let upper = zone.to_uppercase();
    let named = ZONES
        .iter()
        .find(|entry| entry.0 == upper)
        .map(|entry| entry.1);
    let written = named.or_else(|| parse_integer(zone)).unwrap_or(0);
    // Rust's division truncates toward zero, so a negative offset keeps Python's sign times magnitude
    written
        .checked_div(100)?
        .checked_mul(3600)?
        .checked_add(written.checked_rem(100)?.checked_mul(60)?)
}

// Python's int() takes a signed run of digits; a magnitude past i64 saturates, and no decision reads past 60
fn parse_integer(header: &str) -> Option<i64> {
    let negative = header.starts_with('-');
    let digits = header.strip_prefix(['-', '+']).unwrap_or(header);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let mut magnitude: u64 = 0;
    for digit in digits.bytes() {
        magnitude = magnitude
            .saturating_mul(10)
            .saturating_add(u64::from(digit.saturating_sub(b'0')));
    }
    let limit = i64::MAX.unsigned_abs();
    if negative {
        if magnitude > limit {
            return Some(i64::MIN);
        }
        let value = i64::try_from(magnitude).ok()?;
        return Some(value.saturating_neg());
    }
    let value = i64::try_from(magnitude.min(limit)).ok()?;
    Some(value)
}
