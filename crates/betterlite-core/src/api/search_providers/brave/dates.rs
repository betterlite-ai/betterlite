use chrono::{DateTime, Datelike, NaiveDate, Utc};

mod fuzzy;

// the same flags dateutil's parser takes, so an otherwise ambiguous date reads the same way
#[derive(Debug, Clone, Copy, Default)]
pub struct DateFlags {
    pub dayfirst: bool,
    pub yearfirst: bool,
}

// the JSON a Brave reply can put in a date field, kept as the source sees it
#[derive(Debug, Clone)]
pub enum DateValue {
    Text(String),
    Decimal(f64),
    // a value to_yyyy_mm_dd answers null for, with whether the source's `x or y` keeps it
    Ignored(bool),
}

impl DateValue {
    // a JSON string, a number literal, or a value to_yyyy_mm_dd answers null for
    #[must_use]
    pub fn from_literal(literal: &str) -> Self {
        let trimmed = literal.trim();
        match trimmed.as_bytes().first() {
            Some(b'"') => serde_json::from_str(trimmed).map_or(Self::Ignored(false), Self::Text),
            Some(b't') => Self::Ignored(true),
            Some(b'[' | b'{') => {
                let end = trimmed.len().saturating_sub(1);
                let filled = trimmed
                    .get(1..end)
                    .is_some_and(|inner| !inner.trim().is_empty());
                Self::Ignored(filled)
            }
            None | Some(b'f' | b'n') => Self::Ignored(false),
            Some(_) => number_value(trimmed),
        }
    }

    // the source's `page_age or age` keeps the first member its truthiness accepts
    #[must_use]
    pub fn is_filled(&self) -> bool {
        match self {
            Self::Text(text) => !text.is_empty(),
            Self::Decimal(number) => *number != 0.0,
            Self::Ignored(filled) => *filled,
        }
    }
}

fn number_value(literal: &str) -> DateValue {
    if literal.contains(['.', 'e', 'E']) {
        return literal
            .parse::<f64>()
            .map_or(DateValue::Ignored(false), DateValue::Decimal);
    }
    // a numeric zero is falsey, while the string "0" reaches the timestamp branch
    if literal.bytes().all(|byte| byte == b'0' || byte == b'-') {
        return DateValue::Ignored(false);
    }
    DateValue::Text(literal.to_owned())
}

#[must_use]
pub fn to_yyyy_mm_dd(value: &DateValue, today: NaiveDate, flags: DateFlags) -> Option<String> {
    let date = match value {
        DateValue::Ignored(_) => return None,
        DateValue::Text(text) => text_date(text, today, flags)?,
        // the source reads a float through str(), and an infinity has no timestamp
        DateValue::Decimal(number) => {
            if *number == 0.0 || !number.is_finite() {
                return None;
            }
            unix_date(&format!("{number}"))?
        }
    };
    Some(date.format("%Y-%m-%d").to_string())
}

fn text_date(text: &str, today: NaiveDate, flags: DateFlags) -> Option<NaiveDate> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if is_unix_timestamp(trimmed) {
        return unix_date(trimmed);
    }
    // a bare YYYY-M-D always reads year first, whatever the caller's flags say
    let effective = if is_iso_ymd(trimmed) {
        DateFlags {
            dayfirst: false,
            yearfirst: true,
        }
    } else {
        flags
    };
    fuzzy::parse_date(trimmed, today, effective)
}

fn is_unix_timestamp(text: &str) -> bool {
    let body = text.strip_prefix('-').unwrap_or(text);
    let (integer, fraction) = split_decimal(body);
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    digits(integer) && fraction.is_none_or(digits)
}

fn split_decimal(text: &str) -> (&str, Option<&str>) {
    match text.split_once(['.', ',']) {
        Some((integer, fraction)) => (integer, Some(fraction)),
        None => (text, None),
    }
}

fn is_iso_ymd(text: &str) -> bool {
    let parts: Vec<&str> = text.split(['-', '/']).collect();
    let [year, month, day] = parts.as_slice() else {
        return false;
    };
    if year.len() != 4 || !year.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }
    [month, day].into_iter().all(|part| {
        !part.is_empty() && part.len() <= 2 && part.bytes().all(|byte| byte.is_ascii_digit())
    })
}

fn unix_date(text: &str) -> Option<NaiveDate> {
    let value: f64 = text.replace(',', ".").parse().ok()?;
    // above 1e11 the source divides by 1000.0, and the float is the instant it then reads
    let seconds = if value > 1e11 || value < -1e11 {
        value / 1000.0
    } else {
        value
    };
    // Rust has no checked float-to-integer conversion, and an out-of-range value fails the parse
    let whole: i64 = format!("{:.0}", seconds.floor()).parse().ok()?;
    let moment = DateTime::<Utc>::from_timestamp(whole, 0)?;
    let date = moment.date_naive();
    // datetime.fromtimestamp refuses a year outside 1..=9999, and chrono reads it anyway
    (1..=9999).contains(&date.year()).then_some(date)
}
