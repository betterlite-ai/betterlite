use chrono::{DateTime, NaiveDate, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateFormats {
    SecondsOnly,
    SecondsOrDateOnly,
}

impl DateFormats {
    #[must_use]
    pub const fn expected(self) -> &'static str {
        match self {
            Self::SecondsOnly => "'YYYY-MM-DD HH:MM:SS'",
            Self::SecondsOrDateOnly => "'YYYY-MM-DD' or 'YYYY-MM-DD HH:MM:SS'",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("Invalid date format: {value}. Expected: {expected}")]
pub struct DateFormatError {
    pub value: String,
    pub expected: &'static str,
}

pub fn parse_date(text: &str, formats: DateFormats) -> Result<DateTime<Utc>, DateFormatError> {
    let trimmed = text.trim();
    let seconds = parse_seconds(trimmed);
    let date_only = match formats {
        DateFormats::SecondsOnly => None,
        DateFormats::SecondsOrDateOnly => parse_date_only(trimmed),
    };
    let Some(parsed) = seconds.or(date_only) else {
        return Err(DateFormatError {
            value: trimmed.to_owned(),
            expected: formats.expected(),
        });
    };
    Ok(parsed)
}

// strptime's %Y-%m-%d %H:%M:%S: four-digit year, 1-2 digit fields, whitespace for the format's space.
fn parse_seconds(text: &str) -> Option<DateTime<Utc>> {
    let (date_text, time_text) = split_date_and_time(text)?;
    let date = parse_date_fields(date_text)?;
    let mut parts = time_text.split(':');
    let (Some(hour_text), Some(minute_text), Some(second_text)) =
        (parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    if parts.next().is_some() {
        return None;
    }
    let naive = date.and_hms_opt(
        number(hour_text, 2, 0, 23)?,
        number(minute_text, 2, 0, 59)?,
        number(second_text, 2, 0, 59)?,
    )?;
    Some(DateTime::from_naive_utc_and_offset(naive, Utc))
}

// %d may put a space before a one-digit day, so the last whitespace run, not the first, splits date from time.
fn split_date_and_time(text: &str) -> Option<(&str, &str)> {
    let (date_text, time_text) = text.rsplit_once(char::is_whitespace)?;
    Some((date_text.trim_end(), time_text.trim_start()))
}

fn parse_date_only(text: &str) -> Option<DateTime<Utc>> {
    let date = parse_date_fields(text)?;
    let midnight = date.and_hms_opt(0, 0, 0)?;
    Some(DateTime::from_naive_utc_and_offset(midnight, Utc))
}

fn parse_date_fields(text: &str) -> Option<NaiveDate> {
    let mut parts = text.split('-');
    let (Some(year), Some(month), Some(day)) = (parts.next(), parts.next(), parts.next()) else {
        return None;
    };
    if parts.next().is_some() {
        return None;
    }
    NaiveDate::from_ymd_opt(
        year_number(year)?,
        number(month, 2, 1, 12)?,
        day_number(day)?,
    )
}

// strptime's %Y matches exactly four digits, and Python's datetime has no year 0
fn year_number(text: &str) -> Option<i32> {
    if text.len() != 4 || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let value = text.parse::<i32>().ok()?;
    (value >= 1).then_some(value)
}

// strptime's %d also takes one space before a single-digit day.
fn day_number(text: &str) -> Option<u32> {
    let Some(rest) = text.strip_prefix(' ') else {
        return number(text, 2, 1, 31);
    };
    number(rest, 1, 1, 9)
}

fn number(text: &str, max_digits: usize, minimum: u32, maximum: u32) -> Option<u32> {
    if text.len() > max_digits || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let value = text.parse::<u32>().ok()?;
    (minimum..=maximum).contains(&value).then_some(value)
}
