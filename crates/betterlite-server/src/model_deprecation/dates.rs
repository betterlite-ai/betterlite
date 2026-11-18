use chrono::{DateTime, NaiveDate, NaiveDateTime, Weekday};

use betterlite_models::decisions::Json;

#[must_use]
pub fn decode_deprecation_date(text: &str) -> Option<NaiveDate> {
    // Python's str.strip also trims U+001C to U+001F, which char::is_whitespace leaves out
    let trimmed = text.trim_matches(|character: char| {
        character.is_whitespace() || matches!(character, '\u{1c}'..='\u{1f}')
    });
    match trimmed.len() {
        6 => parse_basic_week(trimmed),
        8 => parse_basic_calendar(trimmed)
            .or_else(|| parse_extended_week(trimmed))
            .or_else(|| parse_basic_week_day(trimmed)),
        10 => parse_extended_calendar(trimmed).or_else(|| parse_extended_week_day(trimmed)),
        _ => None,
    }
}

#[must_use]
pub fn parse_json_date(raw: Option<&Json>) -> Option<NaiveDate> {
    match raw {
        Some(Json::Text(text)) => decode_deprecation_date(text),
        Some(Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_))
        | None => None,
    }
}

fn decimal(byte: u8) -> Option<u32> {
    match byte {
        b'0'..=b'9' => Some(u32::from(byte.checked_sub(b'0')?)),
        _ => None,
    }
}

fn number(digits: &[u8]) -> Option<u32> {
    let mut value = 0_u32;
    for digit in digits {
        value = value.checked_mul(10)?.checked_add(decimal(*digit)?)?;
    }
    Some(value)
}

fn calendar_year(digits: &[u8]) -> Option<i32> {
    let year = i32::try_from(number(digits)?).ok()?;
    if year == 0 {
        return None;
    }
    Some(year)
}

const fn iso_weekday(iso: u32) -> Option<Weekday> {
    match iso {
        1 => Some(Weekday::Mon),
        2 => Some(Weekday::Tue),
        3 => Some(Weekday::Wed),
        4 => Some(Weekday::Thu),
        5 => Some(Weekday::Fri),
        6 => Some(Weekday::Sat),
        7 => Some(Weekday::Sun),
        _ => None,
    }
}

fn parse_extended_calendar(text: &str) -> Option<NaiveDate> {
    let bytes: [u8; 10] = text.as_bytes().try_into().ok()?;
    let [y0, y1, y2, y3, b'-', m0, m1, b'-', d0, d1] = bytes;
    NaiveDate::from_ymd_opt(
        calendar_year(&[y0, y1, y2, y3])?,
        number(&[m0, m1])?,
        number(&[d0, d1])?,
    )
}

fn parse_basic_calendar(text: &str) -> Option<NaiveDate> {
    let bytes: [u8; 8] = text.as_bytes().try_into().ok()?;
    let [y0, y1, y2, y3, m0, m1, d0, d1] = bytes;
    NaiveDate::from_ymd_opt(
        calendar_year(&[y0, y1, y2, y3])?,
        number(&[m0, m1])?,
        number(&[d0, d1])?,
    )
}

fn parse_extended_week_day(text: &str) -> Option<NaiveDate> {
    let bytes: [u8; 10] = text.as_bytes().try_into().ok()?;
    let [y0, y1, y2, y3, b'-', b'W', w0, w1, b'-', d] = bytes;
    NaiveDate::from_isoywd_opt(
        calendar_year(&[y0, y1, y2, y3])?,
        number(&[w0, w1])?,
        iso_weekday(decimal(d)?)?,
    )
}

fn parse_basic_week_day(text: &str) -> Option<NaiveDate> {
    let bytes: [u8; 8] = text.as_bytes().try_into().ok()?;
    let [y0, y1, y2, y3, b'W', w0, w1, d] = bytes;
    NaiveDate::from_isoywd_opt(
        calendar_year(&[y0, y1, y2, y3])?,
        number(&[w0, w1])?,
        iso_weekday(decimal(d)?)?,
    )
}

fn parse_extended_week(text: &str) -> Option<NaiveDate> {
    let bytes: [u8; 8] = text.as_bytes().try_into().ok()?;
    let [y0, y1, y2, y3, b'-', b'W', w0, w1] = bytes;
    NaiveDate::from_isoywd_opt(
        calendar_year(&[y0, y1, y2, y3])?,
        number(&[w0, w1])?,
        Weekday::Mon,
    )
}

fn parse_basic_week(text: &str) -> Option<NaiveDate> {
    let bytes: [u8; 6] = text.as_bytes().try_into().ok()?;
    let [y0, y1, y2, y3, b'W', w0, w1] = bytes;
    NaiveDate::from_isoywd_opt(
        calendar_year(&[y0, y1, y2, y3])?,
        number(&[w0, w1])?,
        Weekday::Mon,
    )
}

#[must_use]
pub fn decode_datetime_date(text: &str) -> Option<NaiveDate> {
    match DateTime::parse_from_rfc3339(text) {
        Ok(moment) => Some(moment.date_naive()),
        Err(_) => NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S")
            .ok()
            .map(|moment| moment.date_naive()),
    }
}
