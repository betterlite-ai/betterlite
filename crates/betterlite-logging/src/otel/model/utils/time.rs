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
    if !valid_formatted_datetime(input) {
        return None;
    }
    let format = if input.contains('.') {
        "%Y-%m-%d %H:%M:%S%.f"
    } else {
        "%Y-%m-%d %H:%M:%S"
    };
    NaiveDateTime::parse_from_str(input, format).ok()
}

fn valid_formatted_datetime(input: &str) -> bool {
    let (base, fraction) = input
        .split_once('.')
        .map_or((input, None), |(base, fraction)| (base, Some(fraction)));
    if base.len() != 19 {
        return false;
    }
    let bytes = base.as_bytes();
    let separators = [(4, b'-'), (7, b'-'), (10, b' '), (13, b':'), (16, b':')];
    if separators
        .iter()
        .any(|(position, expected)| bytes.get(*position) != Some(expected))
    {
        return false;
    }
    if bytes
        .iter()
        .enumerate()
        .any(|(position, byte)| !matches!(position, 4 | 7 | 10 | 13 | 16) && !byte.is_ascii_digit())
    {
        return false;
    }
    if bytes.get(17..19) == Some(b"60") {
        return false;
    }
    fraction.is_none_or(|digits| {
        (1..=6).contains(&digits.len()) && digits.bytes().all(|byte| byte.is_ascii_digit())
    })
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
