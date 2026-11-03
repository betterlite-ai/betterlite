use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeZone as _};
use rust_decimal::Decimal;

#[path = "time/tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateTimeInput {
    Zoned(DateTime<FixedOffset>),
    Naive(NaiveDateTime),
}

pub trait LocalTimeResolver {
    fn resolve(&self, local: NaiveDateTime) -> Option<DateTime<FixedOffset>>;
}

#[must_use]
pub fn to_ns<R>(
    value: Option<&Decimal>,
    datetime: Option<DateTimeInput>,
    resolver: &R,
) -> Option<i128>
where
    R: LocalTimeResolver,
{
    if let Some(input_datetime) = datetime {
        let decimal = timestamp_seconds(input_datetime, resolver)?;
        return decimal
            .checked_mul(Decimal::from(1_000_000_000_i64))?
            .trunc()
            .to_i128();
    }
    value?
        .checked_mul(Decimal::from(1_000_000_000_i64))?
        .trunc()
        .to_i128()
}

#[must_use]
pub fn to_seconds<R>(
    value: Option<&Decimal>,
    datetime: Option<DateTimeInput>,
    text: Option<&str>,
    resolver: &R,
) -> Option<Decimal>
where
    R: LocalTimeResolver,
{
    if let Some(input_datetime) = datetime {
        return timestamp_seconds(input_datetime, resolver);
    }
    if let Some(epoch_value) = value {
        return Some(*epoch_value);
    }
    let input_text = text?;
    let parsed = parse_formatted_datetime(input_text)?;
    timestamp_seconds(DateTimeInput::Naive(parsed), resolver)
}

fn parse_formatted_datetime(input: &str) -> Option<NaiveDateTime> {
    let normalized = normalize_formatted_datetime(input)?;
    let format = if normalized.contains('.') {
        "%Y-%m-%d %H:%M:%S%.f"
    } else {
        "%Y-%m-%d %H:%M:%S"
    };
    NaiveDateTime::parse_from_str(&normalized, format).ok()
}

fn normalize_formatted_datetime(input: &str) -> Option<String> {
    if input.len() > 64 {
        return None;
    }
    let (year, month, day, date_time_separator) = parse_date_prefix(input)?;
    if !date_time_separator
        .chars()
        .next()
        .is_some_and(char::is_whitespace)
    {
        return None;
    }
    let time_text = date_time_separator.trim_start_matches(char::is_whitespace);
    if time_text.is_empty() {
        return None;
    }
    let (clock, fraction) = time_text
        .split_once('.')
        .map_or((time_text, None), |(clock, fraction)| {
            (clock, Some(fraction))
        });
    let (hour, minute, second) = parse_clock_fields(clock)?;
    if second == 60 {
        return None;
    }
    let mut normalized = format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}");
    if let Some(fraction_text) = fraction {
        if !valid_fraction(fraction_text) {
            return None;
        }
        normalized.push('.');
        normalized.push_str(fraction_text);
    }
    Some(normalized)
}

fn parse_date_prefix(input: &str) -> Option<(u32, u32, u32, &str)> {
    let (year_text, remainder) = input.split_once('-')?;
    let (month_text, day_and_time) = remainder.split_once('-')?;
    if year_text.len() != 4 || !year_text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let year = year_text.parse::<u32>().ok()?;
    if year == 0 {
        return None;
    }
    let month = parse_clock_component(month_text)?;
    let (day, date_time_separator) = parse_day_prefix(day_and_time)?;
    Some((year, month, day, date_time_separator))
}

fn parse_day_prefix(input: &str) -> Option<(u32, &str)> {
    let mut characters = input.chars();
    let first = characters.next()?;
    if first == ' ' {
        let day = characters.next()?;
        if !day.is_ascii_digit() {
            return None;
        }
        return Some((day.to_digit(10)?, characters.as_str()));
    }
    if !first.is_ascii_digit() {
        return None;
    }
    let mut day_text = String::from(first);
    if characters
        .clone()
        .next()
        .is_some_and(|character| character.is_ascii_digit())
    {
        day_text.push(characters.next()?);
    }
    Some((parse_clock_component(&day_text)?, characters.as_str()))
}

fn parse_clock_fields(input: &str) -> Option<(u32, u32, u32)> {
    let mut fields = input.split(':');
    let hour = fields.next()?;
    let minute = fields.next()?;
    let second = fields.next()?;
    if fields.next().is_some() {
        return None;
    }
    Some((
        parse_clock_component(hour)?,
        parse_clock_component(minute)?,
        parse_clock_component(second)?,
    ))
}

fn parse_clock_component(input: &str) -> Option<u32> {
    if !(1..=2).contains(&input.len()) || !input.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    input.parse::<u32>().ok()
}

fn valid_fraction(input: &str) -> bool {
    if !(1..=6).contains(&input.len()) {
        return false;
    }
    input.bytes().all(|byte| byte.is_ascii_digit())
}

fn timestamp_seconds<R>(value: DateTimeInput, resolver: &R) -> Option<Decimal>
where
    R: LocalTimeResolver,
{
    let zoned = match value {
        DateTimeInput::Zoned(zoned) => zoned,
        DateTimeInput::Naive(local) => resolver.resolve(local)?,
    };
    let seconds = Decimal::from(zoned.timestamp());
    let nanos = Decimal::new(i64::from(zoned.timestamp_subsec_nanos()), 9);
    seconds.checked_add(nanos)
}

pub struct FixedLocalTimeResolver {
    pub offset: FixedOffset,
}

impl LocalTimeResolver for FixedLocalTimeResolver {
    fn resolve(&self, local: NaiveDateTime) -> Option<DateTime<FixedOffset>> {
        self.offset.from_local_datetime(&local).single()
    }
}

use rust_decimal::prelude::ToPrimitive as _;
