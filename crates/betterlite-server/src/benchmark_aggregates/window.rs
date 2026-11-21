use chrono::Datelike as _;
use chrono::{Days, NaiveDate};
use thiserror::Error;

pub const DEFAULT_WINDOW_DAYS: u64 = 30;

const FIRST_CALENDAR_YEAR: i32 = 1;
const LAST_CALENDAR_YEAR: i32 = 9999;
const YEAR_DIGITS: usize = 4;

// The resolved inclusive window: response dates as Python prints them, query bounds as isoformat
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BenchmarkWindow {
    pub start_date: String,
    pub end_date: String,
    pub query_start: String,
    pub query_end: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WindowError {
    #[error("Invalid date format: {value}. Expected: 'YYYY-MM-DD'")]
    InvalidDate { value: String },
    #[error("end_date must not be earlier than start_date")]
    ReversedRange,
    #[error("the requested window runs past the calendar these dates can represent")]
    CalendarRange,
}

impl BenchmarkWindow {
    #[must_use]
    pub fn resolve(
        start_date: Option<&str>,
        end_date: Option<&str>,
        today: NaiveDate,
    ) -> Result<Self, WindowError> {
        let end = match end_date {
            Some(value) => parse_day(value)?,
            None => today,
        };
        let start = match start_date {
            Some(value) => parse_day(value)?,
            None => end
                .checked_sub_days(Days::new(DEFAULT_WINDOW_DAYS))
                .filter(|date| date.year() >= FIRST_CALENDAR_YEAR)
                .ok_or(WindowError::CalendarRange)?,
        };
        if end < start {
            return Err(WindowError::ReversedRange);
        }
        let after_end = end
            .checked_add_days(Days::new(1))
            .filter(|date| date.year() <= LAST_CALENDAR_YEAR)
            .ok_or(WindowError::CalendarRange)?;
        // strftime("%Y-%m-%d") on glibc leaves a year below 1000 unpadded
        Ok(Self {
            start_date: format!("{}-{:02}-{:02}", start.year(), start.month(), start.day()),
            end_date: format!("{}-{:02}-{:02}", end.year(), end.month(), end.day()),
            query_start: instant_text(start),
            query_end: instant_text(after_end),
        })
    }
}

fn parse_day(value: &str) -> Result<NaiveDate, WindowError> {
    let invalid = || WindowError::InvalidDate {
        value: value.to_owned(),
    };
    let mut parts = value.split('-');
    let (Some(year), Some(month), Some(day), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(invalid());
    };
    let parsed_year = parse_part(year, YEAR_DIGITS, YEAR_DIGITS)
        .filter(|candidate| (FIRST_CALENDAR_YEAR..=LAST_CALENDAR_YEAR).contains(candidate))
        .ok_or_else(invalid)?;
    let parsed_month = parse_part(month, 1, 2).ok_or_else(invalid)?;
    let parsed_day = parse_day_part(day).ok_or_else(invalid)?;
    NaiveDate::from_ymd_opt(
        parsed_year,
        u32::try_from(parsed_month).unwrap_or_default(),
        u32::try_from(parsed_day).unwrap_or_default(),
    )
    .ok_or_else(invalid)
}

// Python's strptime reads %d as a plain digit, two digits, or one digit behind a space
fn parse_day_part(text: &str) -> Option<i32> {
    match text.strip_prefix(' ') {
        Some(digits) => parse_part(digits, 1, 1),
        None => parse_part(text, 1, 2),
    }
}

fn parse_part(text: &str, min_digits: usize, max_digits: usize) -> Option<i32> {
    if text.len() < min_digits || text.len() > max_digits {
        return None;
    }
    if !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse::<i32>().ok()
}

// datetime.isoformat() of midnight UTC, the bound the rollup query binds
fn instant_text(date: NaiveDate) -> String {
    format!(
        "{:04}-{:02}-{:02}T00:00:00",
        date.year(),
        date.month(),
        date.day()
    )
}
