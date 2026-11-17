use betterlite_models::decisions::Json;
use betterlite_models::savings_evidence::billed_at::decode_billed_at;
use chrono::{DateTime, FixedOffset, TimeDelta, Utc};

// the injected wall clock and the local zone a naive timestamp is read in
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clock {
    pub now: i64,
    pub zone: FixedOffset,
}

// the fromisoformat grammar is the shared decode_billed_at; int() truncates the seconds and a value without an offset is read in the machine's zone
#[must_use]
pub fn parse_timestamp(value: &Json, zone: FixedOffset) -> Option<i64> {
    let Json::Text(text) = value else {
        return None;
    };
    let parsed = decode_billed_at(text)?;
    let normalised = text.replace('Z', "+00:00");
    let chars: Vec<char> = normalised.chars().collect();
    let length = date_length(&chars);
    // CPython's C parser rejects a date left with its separator and no time, which the shared parser reads as midnight
    if chars.len() == length.saturating_add(1) {
        return None;
    }
    let instant = if has_offset(&chars, length) {
        parsed
    } else {
        parsed.checked_sub_signed(TimeDelta::try_seconds(i64::from(zone.local_minus_utc()))?)?
    };
    truncate_toward_zero(instant)
}

// int() truncates toward zero while chrono's timestamp() floors a negative instant
fn truncate_toward_zero(instant: DateTime<Utc>) -> Option<i64> {
    let seconds = instant.timestamp();
    if seconds < 0 && instant.timestamp_subsec_nanos() > 0 {
        return seconds.checked_add(1);
    }
    Some(seconds)
}

// fromisoformat finds an offset by a sign in the time part, which starts right after the date and its one separator
fn has_offset(chars: &[char], length: usize) -> bool {
    chars
        .get(length.saturating_add(1)..)
        .is_some_and(|time| time.iter().any(|character| matches!(character, '+' | '-')))
}

// mirrors where CPython's fromisoformat ends the date and starts the time: 7, 8 or 10 characters by the date form
fn date_length(chars: &[char]) -> usize {
    match chars.get(4) {
        Some('-') if chars.get(5) == Some(&'W') => week_dash_length(chars),
        Some('-') => 10,
        Some('W') => basic_week_length(chars),
        _ => 8,
    }
}

// CPython ends a week date at the dash after the week only when no hour digit comes two places on, and otherwise at the day sign
fn week_dash_length(chars: &[char]) -> usize {
    if chars.get(8) != Some(&'-') || chars.get(10).is_some_and(char::is_ascii_digit) {
        return 8;
    }
    10
}

// a basic week date ends at 7 characters, or 8 when a day digit follows the week; CPython's longer tails start the hour with a digit, so either end reads the same
fn basic_week_length(chars: &[char]) -> usize {
    if chars.get(7).is_some_and(char::is_ascii_digit) {
        8
    } else {
        7
    }
}
