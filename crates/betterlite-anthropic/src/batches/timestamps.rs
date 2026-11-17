use betterlite_models::decisions::Json;
use chrono::{FixedOffset, NaiveDate, NaiveTime};

// the injected wall clock and the local zone a naive timestamp is read in
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clock {
    pub now: i64,
    pub zone: FixedOffset,
}

// LiteLLM parses with datetime.fromisoformat after replacing Z, keeps whole seconds, and drops every failure
#[must_use]
pub fn parse_timestamp(value: &Json, zone: FixedOffset) -> Option<i64> {
    let Json::Text(text) = value else {
        return None;
    };
    if text.is_empty() {
        return None;
    }
    let normalised = text.replace('Z', "+00:00");
    let (date, clock) = match normalised.split_once(['T', ' ']) {
        Some((date, clock)) if !clock.is_empty() => (date, clock),
        Some(_) => return None,
        None => (normalised.as_str(), ""),
    };
    let date = parse_date(date)?;
    let (time, offset) = parse_time(clock)?;
    let utc = date.and_time(time).and_utc();
    let seconds = offset.unwrap_or_else(|| zone.local_minus_utc());
    let whole = utc.timestamp().checked_sub(i64::from(seconds))?;
    let nanos = utc.timestamp_subsec_nanos();
    if whole < 0 && nanos > 0 {
        return whole.checked_add(1);
    }
    Some(whole)
}

fn parse_date(text: &str) -> Option<NaiveDate> {
    let bytes = text.as_bytes();
    let (year, month, day) =
        if text.len() == 10 && bytes[4] == b'-' && bytes[7] == b'-' && text.is_ascii() {
            (&text[..4], &text[5..7], &text[8..10])
        } else if text.len() == 8 && text.bytes().all(|byte| byte.is_ascii_digit()) {
            (&text[..4], &text[4..6], &text[6..8])
        } else {
            return None;
        };
    let year: i32 = year.parse().ok()?;
    if !(1..=9999).contains(&year) {
        return None;
    }
    NaiveDate::from_ymd_opt(year, month.parse().ok()?, day.parse().ok()?)
}

fn parse_time(text: &str) -> Option<(NaiveTime, Option<i32>)> {
    if text.is_empty() {
        return Some((NaiveTime::MIN, None));
    }
    let (clock, offset) = match text.rfind(['+', '-']) {
        Some(index) => (&text[..index], Some(parse_offset(&text[index..])?)),
        None => (text, None),
    };
    Some((parse_clock(clock)?, offset))
}

fn parse_clock(text: &str) -> Option<NaiveTime> {
    let (main, micros) = match text.split_once(['.', ',']) {
        Some((main, fraction)) => (main, parse_fraction(fraction)?),
        None => (text, 0),
    };
    let mut parts = main.split(':');
    let hour = digits2(parts.next()?)?;
    let minute = digits2(parts.next()?)?;
    let second = match parts.next() {
        Some(part) => digits2(part)?,
        None => 0,
    };
    if parts.next().is_some() {
        return None;
    }
    let nanos = micros.checked_mul(1_000)?;
    NaiveTime::from_hms_nano_opt(hour, minute, second, nanos)
}

fn parse_fraction(text: &str) -> Option<u32> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let mut micros: u32 = 0;
    for byte in text.bytes().take(6) {
        micros = micros
            .checked_mul(10)?
            .checked_add(u32::from(byte - b'0'))?;
    }
    for _ in text.len().min(6)..6 {
        micros = micros.checked_mul(10)?;
    }
    Some(micros)
}

fn parse_offset(text: &str) -> Option<i32> {
    let (sign, rest) = match text.as_bytes().first()? {
        b'+' => (1_i32, &text[1..]),
        b'-' => (-1_i32, &text[1..]),
        _ => return None,
    };
    if !rest.is_ascii() {
        return None;
    }
    let (hours, minutes) = match rest.split_once(':') {
        Some((hours, minutes)) => (digits2(hours)?, digits2(minutes)?),
        None if rest.len() == 4 => (digits2(&rest[..2])?, digits2(&rest[2..])?),
        None => (digits2(rest)?, 0),
    };
    if hours > 23 || minutes > 59 {
        return None;
    }
    let hour_seconds = i32::try_from(hours).ok()?.checked_mul(3600)?;
    let minute_seconds = i32::try_from(minutes).ok()?.checked_mul(60)?;
    let total = hour_seconds.checked_add(minute_seconds)?;
    sign.checked_mul(total)
}

fn digits2(text: &str) -> Option<u32> {
    if text.len() != 2 || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}
