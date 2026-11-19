use std::cmp::Ordering;

use chrono::{DateTime, Datelike as _, NaiveDate, Utc};

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
    Integer(String),
    Decimal(f64),
    // a value to_yyyy_mm_dd answers null for, with whether the source's `x or y` keeps it
    Ignored(bool),
}

impl DateValue {
    // a JSON string, a number literal, or a value to_yyyy_mm_dd answers null for
    #[must_use]
    pub fn from_literal(literal: &str) -> Self {
        let literal = literal.trim();
        match literal.as_bytes().first() {
            Some(b'"') => serde_json::from_str(literal).map_or(Self::Ignored(false), Self::Text),
            Some(b't') => Self::Ignored(true),
            Some(b'[' | b'{') => Self::Ignored(has_members(literal)),
            Some(b'f' | b'n') => Self::Ignored(false),
            Some(_) => number_value(literal),
            None => Self::Ignored(false),
        }
    }

    // the source's `page_age or age` keeps the first member its truthiness accepts
    #[must_use]
    pub fn is_filled(&self) -> bool {
        match self {
            Self::Text(text) => !text.is_empty(),
            Self::Integer(text) => text.bytes().any(|byte| byte != b'0'),
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
    DateValue::Integer(literal.to_owned())
}

fn has_members(literal: &str) -> bool {
    let end = literal.len().saturating_sub(1);
    literal
        .get(1..end)
        .is_some_and(|inner| !inner.trim().is_empty())
}

#[must_use]
pub fn to_yyyy_mm_dd(value: &DateValue, today: NaiveDate, flags: DateFlags) -> Option<String> {
    let date = match value {
        DateValue::Ignored(_) => return None,
        DateValue::Text(text) => text_date(text, today, flags)?,
        DateValue::Integer(text) => unix_date(text)?,
        DateValue::Decimal(number) => decimal_date(*number)?,
    };
    Some(format!(
        "{:04}-{:02}-{:02}",
        date.year(),
        date.month(),
        date.day()
    ))
}

fn decimal_date(number: f64) -> Option<NaiveDate> {
    if number == 0.0 || !(1e-4..1e16).contains(&number.abs()) {
        return None;
    }
    unix_date(&format!("{number}"))
}

fn text_date(text: &str, today: NaiveDate, flags: DateFlags) -> Option<NaiveDate> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if is_unix_timestamp(text) {
        return unix_date(text);
    }
    // a bare YYYY-M-D always reads year first, whatever the caller's flags say
    let flags = if is_iso_ymd(text) {
        DateFlags {
            dayfirst: false,
            yearfirst: true,
        }
    } else {
        flags
    };
    fuzzy::parse_date(text, today, flags)
}

fn is_unix_timestamp(text: &str) -> bool {
    let body = text.strip_prefix('-').unwrap_or(text);
    let (integer, fraction) = split_decimal(body);
    !integer.is_empty()
        && integer.bytes().all(|byte| byte.is_ascii_digit())
        && fraction
            .is_none_or(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
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
    let (negative, body) = match text.strip_prefix('-') {
        Some(body) => (true, body),
        None => (false, text),
    };
    let (integer, fraction) = split_decimal(body);
    let integer = integer.trim_start_matches('0');
    let fraction = fraction.unwrap_or_default();
    if integer.len() > 30 {
        return None;
    }
    let (integer, fraction) = if above_threshold(integer, fraction) {
        shift_three(integer, fraction)?
    } else {
        (integer.to_owned(), fraction.to_owned())
    };
    let whole: i128 = if integer.is_empty() {
        0
    } else {
        integer.parse().ok()?
    };
    let fractional = fraction.bytes().any(|byte| byte != b'0');
    let seconds = match (negative, fractional) {
        (true, true) => whole.checked_add(1)?.checked_neg()?,
        (true, false) => whole.checked_neg()?,
        (false, _) => whole,
    };
    let moment = DateTime::<Utc>::from_timestamp(i64::try_from(seconds).ok()?, 0)?;
    Some(moment.date_naive())
}

fn above_threshold(integer: &str, fraction: &str) -> bool {
    const LIMIT: &str = "100000000000";
    match integer.len().cmp(&LIMIT.len()) {
        Ordering::Greater => true,
        Ordering::Less => false,
        Ordering::Equal => match integer.cmp(LIMIT) {
            Ordering::Greater => true,
            Ordering::Less => false,
            Ordering::Equal => fraction.bytes().any(|byte| byte != b'0'),
        },
    }
}

// the source divides milliseconds by 1000.0; the exact decimal shift keeps every digit
fn shift_three(integer: &str, fraction: &str) -> Option<(String, String)> {
    let cut = integer.len().checked_sub(3)?;
    let (head, tail) = integer.split_at_checked(cut)?;
    Some((head.to_owned(), format!("{tail}{fraction}")))
}
