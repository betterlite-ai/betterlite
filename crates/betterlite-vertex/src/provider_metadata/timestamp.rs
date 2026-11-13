use std::str::{Utf8Error, from_utf8};

use jiff::civil;
use jiff::tz::{AmbiguousOffset, TimeZone};
use thiserror::Error;

const MAX_TIMESTAMP_BYTES: usize = 64;
const NANOS_PER_SECOND: i128 = 1_000_000_000;
// CPython's mktime treats the naive value as a UTC epoch and probes localtime() around it
const MIN_YEAR_SECONDS: i128 = -62_135_596_800;
const MAX_YEAR_SECONDS: i128 = 253_402_300_799;
const FOLD_PROBE_SECONDS: i128 = 86_400;

#[derive(Clone, Debug)]
pub struct LocalTimeContext {
    pub zone: TimeZone,
}

#[derive(Debug, Error)]
pub enum VertexTimestampError {
    #[error("the vertex timestamp is {size} bytes, over the {limit} byte limit")]
    TooLong { size: usize, limit: usize },
    #[error("the vertex timestamp is not valid UTF-8")]
    Utf8(#[source] Utf8Error),
    #[error("the vertex timestamp does not match the %Y-%m-%dT%H:%M:%S.%fZ format")]
    Format,
    #[error("the vertex timestamp names an invalid date or time")]
    InvalidCalendar(#[source] jiff::Error),
    #[error("the vertex timestamp is out of the supported range")]
    OutOfRange,
}

#[derive(Clone, Copy)]
struct TimestampParts<'text> {
    year: &'text str,
    month: &'text str,
    day: &'text str,
    hour: &'text str,
    minute: &'text str,
    second: &'text str,
    fraction: &'text str,
}

pub fn decode_vertex_timestamp(
    bytes: &[u8],
    local: &LocalTimeContext,
) -> Result<i64, VertexTimestampError> {
    if bytes.len() > MAX_TIMESTAMP_BYTES {
        return Err(VertexTimestampError::TooLong {
            size: bytes.len(),
            limit: MAX_TIMESTAMP_BYTES,
        });
    }
    let text = from_utf8(bytes).map_err(VertexTimestampError::Utf8)?;
    let civil = civil_from_parts(split_timestamp(text)?)?;
    let epoch = civil::DateTime::new(1970, 1, 1, 0, 0, 0, 0)
        .map_err(VertexTimestampError::InvalidCalendar)?;
    // the source reads a naive datetime in the process's local zone; fold=0 and mktime both take the offset before a transition
    let offset = match local.zone.to_ambiguous_timestamp(civil).offset() {
        AmbiguousOffset::Unambiguous { offset } => offset,
        AmbiguousOffset::Gap { before, .. } | AmbiguousOffset::Fold { before, .. } => before,
    };
    // jiff's Timestamp stops before the last hour of year 9999, but civil arithmetic covers it; dividing truncates toward zero like int()
    let civil_nanos = civil.duration_since(epoch).as_nanos();
    let offset_seconds = i128::from(offset.seconds());
    check_year_range(civil_nanos, offset_seconds)?;
    let offset_nanos = offset_seconds
        .checked_mul(NANOS_PER_SECOND)
        .ok_or(VertexTimestampError::OutOfRange)?;
    let instant_nanos = civil_nanos
        .checked_sub(offset_nanos)
        .ok_or(VertexTimestampError::OutOfRange)?;
    let seconds = instant_nanos
        .checked_div(NANOS_PER_SECOND)
        .ok_or(VertexTimestampError::OutOfRange)?;
    i64::try_from(seconds).map_err(|_error| VertexTimestampError::OutOfRange)
}

// CPython's timestamp() reads localtime() of the civil value taken as UTC and of one day before it, so either probe can name a year outside 1..9999
fn check_year_range(civil_nanos: i128, offset_seconds: i128) -> Result<(), VertexTimestampError> {
    let civil_seconds = civil_nanos
        .checked_div_euclid(NANOS_PER_SECOND)
        .ok_or(VertexTimestampError::OutOfRange)?;
    let local_seconds = civil_seconds
        .checked_add(offset_seconds)
        .ok_or(VertexTimestampError::OutOfRange)?;
    if !(MIN_YEAR_SECONDS..=MAX_YEAR_SECONDS).contains(&local_seconds) {
        return Err(VertexTimestampError::OutOfRange);
    }
    let fold_probe_seconds = civil_seconds
        .checked_sub(FOLD_PROBE_SECONDS)
        .ok_or(VertexTimestampError::OutOfRange)?;
    if fold_probe_seconds < MIN_YEAR_SECONDS {
        return Err(VertexTimestampError::OutOfRange);
    }
    Ok(())
}

fn split_timestamp(text: &str) -> Result<TimestampParts<'_>, VertexTimestampError> {
    let Some((date, rest)) = text.split_once(['T', 't']) else {
        return Err(VertexTimestampError::Format);
    };
    let Some(clock) = rest.strip_suffix(['Z', 'z']) else {
        return Err(VertexTimestampError::Format);
    };
    let Some((time, fraction)) = clock.split_once('.') else {
        return Err(VertexTimestampError::Format);
    };
    let mut date_fields = date.split('-');
    let year = date_fields.next().ok_or(VertexTimestampError::Format)?;
    let month = date_fields.next().ok_or(VertexTimestampError::Format)?;
    let day = date_fields.next().ok_or(VertexTimestampError::Format)?;
    if date_fields.next().is_some() {
        return Err(VertexTimestampError::Format);
    }
    let mut time_fields = time.split(':');
    let hour = time_fields.next().ok_or(VertexTimestampError::Format)?;
    let minute = time_fields.next().ok_or(VertexTimestampError::Format)?;
    let second = time_fields.next().ok_or(VertexTimestampError::Format)?;
    if time_fields.next().is_some() {
        return Err(VertexTimestampError::Format);
    }
    Ok(TimestampParts {
        year,
        month,
        day,
        hour,
        minute,
        second,
        fraction,
    })
}

fn civil_from_parts(parts: TimestampParts<'_>) -> Result<civil::DateTime, VertexTimestampError> {
    let year = parse_digits(parts.year, 4, 4).ok_or(VertexTimestampError::Format)?;
    if year == 0 {
        return Err(VertexTimestampError::Format);
    }
    let month = parse_digits(parts.month, 1, 2).ok_or(VertexTimestampError::Format)?;
    let day = parse_day(parts.day).ok_or(VertexTimestampError::Format)?;
    let hour = parse_hour(parts.hour).ok_or(VertexTimestampError::Format)?;
    let minute = parse_digits(parts.minute, 1, 2).ok_or(VertexTimestampError::Format)?;
    let second = parse_digits(parts.second, 1, 2).ok_or(VertexTimestampError::Format)?;
    let nanosecond = subsec_nanoseconds(parts.fraction)?;
    let year_number = i16::try_from(year).map_err(|_error| VertexTimestampError::OutOfRange)?;
    let month_number = i8::try_from(month).map_err(|_error| VertexTimestampError::OutOfRange)?;
    let day_number = i8::try_from(day).map_err(|_error| VertexTimestampError::OutOfRange)?;
    let hour_number = i8::try_from(hour).map_err(|_error| VertexTimestampError::OutOfRange)?;
    let minute_number = i8::try_from(minute).map_err(|_error| VertexTimestampError::OutOfRange)?;
    let second_number = i8::try_from(second).map_err(|_error| VertexTimestampError::OutOfRange)?;
    civil::DateTime::new(
        year_number,
        month_number,
        day_number,
        hour_number,
        minute_number,
        second_number,
        nanosecond,
    )
    .map_err(VertexTimestampError::InvalidCalendar)
}

fn subsec_nanoseconds(fraction: &str) -> Result<i32, VertexTimestampError> {
    let microsecond_scale = match fraction.len() {
        1 => 100_000,
        2 => 10_000,
        3 => 1_000,
        4 => 100,
        5 => 10,
        _ => 1,
    };
    let micros = parse_digits(fraction, 1, 6)
        .ok_or(VertexTimestampError::Format)?
        .checked_mul(microsecond_scale)
        .ok_or(VertexTimestampError::OutOfRange)?;
    let nanos = micros
        .checked_mul(1_000)
        .ok_or(VertexTimestampError::OutOfRange)?;
    i32::try_from(nanos).map_err(|_error| VertexTimestampError::OutOfRange)
}

// strptime's %d also takes a space before a single digit 1-9, and its %H a space before any single digit
fn parse_day(text: &str) -> Option<u32> {
    let Some(rest) = text.strip_prefix(' ') else {
        return parse_digits(text, 1, 2);
    };
    parse_digits(rest, 1, 1).filter(|day| *day != 0)
}

fn parse_hour(text: &str) -> Option<u32> {
    let Some(rest) = text.strip_prefix(' ') else {
        return parse_digits(text, 1, 2);
    };
    parse_digits(rest, 1, 1)
}

fn parse_digits(text: &str, min: usize, max: usize) -> Option<u32> {
    if !(min..=max).contains(&text.len()) || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}
