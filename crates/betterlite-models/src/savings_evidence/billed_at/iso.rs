use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, Utc, Weekday};

const FRACTION_WIDTH: usize = 6;
const FRACTION_SCALE: [u32; 7] = [0, 100_000, 10_000, 1_000, 100, 10, 1];
const MAX_OFFSET_MICROS: u64 = 86_400_000_000;
const SECONDS_PER_HOUR: i64 = 3600;
const SECONDS_PER_MINUTE: i64 = 60;
const MICROS_PER_SECOND: i64 = 1_000_000;

#[derive(Clone, Copy)]
struct ClockFields {
    hour: u32,
    minute: u32,
    second: u32,
    micro: u32,
}

// Reads `width` ASCII digits starting at `position` and returns the value with the position just past them.
fn take_digits(chars: &[char], position: usize, width: usize) -> Option<(u32, usize)> {
    let end = position.saturating_add(width);
    let slice = chars.get(position..end)?;
    if slice.len() != width || !slice.iter().all(char::is_ascii_digit) {
        return None;
    }
    let value: u32 = slice.iter().collect::<String>().parse().ok()?;
    Some((value, end))
}

// Ports CPython 3.13's datetime.fromisoformat grammar (_pydatetime.py) char by char, since chrono's own format list has no form for a week date, a comma fraction, a bare-hour or seconds-wide offset, or a one-sided basic date, and it accepts a leap second and one-digit fields that fromisoformat rejects.
pub(super) fn parse(chars: &[char]) -> Option<DateTime<Utc>> {
    if chars.len() < 7 {
        return None;
    }
    let separator_at = find_separator(chars)?;
    let date_chars = chars.get(..separator_at)?;
    let date = parse_date(date_chars)?;
    let time_start = separator_at.saturating_add(1);
    let time_chars = chars.get(time_start..).unwrap_or(&[]);
    if time_chars.is_empty() {
        let midnight = ClockFields {
            hour: 0,
            minute: 0,
            second: 0,
            micro: 0,
        };
        return build_instant(date, &midnight, None);
    }
    let (clock, offset) = parse_time(time_chars)?;
    if clock.hour > 23 || clock.minute > 59 || clock.second > 59 || clock.micro > 999_999 {
        return None;
    }
    if offset.is_some_and(|value| value.unsigned_abs() >= MAX_OFFSET_MICROS) {
        return None;
    }
    build_instant(date, &clock, offset)
}

// Mirrors _find_isoformat_datetime_separator: the date form alone decides where the one separator sits, and the character there is never inspected, so any single character, ASCII or not, is fine.
fn find_separator(chars: &[char]) -> Option<usize> {
    if chars.len() == 7 {
        return Some(7);
    }
    if chars.get(4) == Some(&'-') {
        return find_dash_form_separator(chars);
    }
    if chars.get(4) == Some(&'W') {
        return Some(find_basic_week_separator(chars));
    }
    Some(8)
}

fn find_dash_form_separator(chars: &[char]) -> Option<usize> {
    if chars.get(5) != Some(&'W') {
        return Some(10);
    }
    find_week_dash_separator(chars)
}

fn find_week_dash_separator(chars: &[char]) -> Option<usize> {
    let len = chars.len();
    if len <= 8 || chars.get(8) != Some(&'-') {
        return Some(8);
    }
    if len == 9 {
        return None;
    }
    let has_numeric_tail = len > 10 && chars.get(10).is_some_and(char::is_ascii_digit);
    Some(if has_numeric_tail { 8 } else { 10 })
}

fn find_basic_week_separator(chars: &[char]) -> usize {
    let mut idx = 7;
    while chars.get(idx).is_some_and(char::is_ascii_digit) {
        idx = idx.saturating_add(1);
    }
    if idx < 9 {
        return idx;
    }
    if idx & 1 == 0 { 7 } else { 8 }
}

// Mirrors _parse_isoformat_date: YYYY-MM-DD, YYYYMMDD, YYYY-Www[-D] or YYYYWww[D], a dash used everywhere or nowhere, and a year outside 1..=9999 or a day outside its month rejected by chrono.
fn parse_date(chars: &[char]) -> Option<NaiveDate> {
    let len = chars.len();
    if !matches!(len, 7 | 8 | 10) || !chars.iter().all(char::is_ascii) {
        return None;
    }
    let (raw_year, _) = take_digits(chars, 0, 4)?;
    if !(1..=9999).contains(&raw_year) {
        return None;
    }
    let year = i32::try_from(raw_year).ok()?;
    let has_sep = chars.get(4) == Some(&'-');
    let after_year = 4_usize.saturating_add(usize::from(has_sep));
    if chars.get(after_year) == Some(&'W') {
        let after_marker = after_year.saturating_add(1);
        return parse_week_date(chars, after_marker, year, has_sep);
    }
    parse_month_day_date(chars, after_year, year, has_sep)
}

// Reads a dash-gated numeric component: the dash at `position` must match `has_sep` exactly once, then `width` digits follow it.
fn read_separated_component(
    chars: &[char],
    position: usize,
    has_sep: bool,
    width: usize,
) -> Option<(u32, usize)> {
    if (chars.get(position) == Some(&'-')) != has_sep {
        return None;
    }
    let value_position = position.saturating_add(usize::from(has_sep));
    take_digits(chars, value_position, width)
}

fn parse_week_date(chars: &[char], position: usize, year: i32, has_sep: bool) -> Option<NaiveDate> {
    let (week, after_week) = take_digits(chars, position, 2)?;
    let day = if chars.get(after_week).is_none() {
        1
    } else {
        read_separated_component(chars, after_week, has_sep, 1)?.0
    };
    let weekday = Weekday::try_from(u8::try_from(day.checked_sub(1)?).ok()?).ok()?;
    NaiveDate::from_isoywd_opt(year, week, weekday)
}

fn parse_month_day_date(
    chars: &[char],
    position: usize,
    year: i32,
    has_sep: bool,
) -> Option<NaiveDate> {
    let (month, after_month) = take_digits(chars, position, 2)?;
    let (day, _) = read_separated_component(chars, after_month, has_sep, 2)?;
    NaiveDate::from_ymd_opt(year, month, day)
}

// Mirrors _parse_isoformat_time: a clock, then an optional offset found by the first '-' or '+' (fromisoformat's own 'Z' branch is dead here, since the caller already replaced every Z with +00:00).
fn parse_time(chars: &[char]) -> Option<(ClockFields, Option<i64>)> {
    if chars.len() < 2 || !chars.iter().all(char::is_ascii) {
        return None;
    }
    let maybe_sign_index = chars
        .iter()
        .position(|&character| character == '-')
        .or_else(|| chars.iter().position(|&character| character == '+'));
    let Some(sign_index) = maybe_sign_index else {
        return Some((parse_clock(chars)?, None));
    };
    let clock_chars = chars.get(..sign_index)?;
    let clock = parse_clock(clock_chars)?;
    let offset_start = sign_index.saturating_add(1);
    let offset_chars = chars.get(offset_start..).unwrap_or(&[]);
    if matches!(offset_chars.len(), 0 | 1 | 3) {
        return None;
    }
    let offset_clock = parse_clock(offset_chars)?;
    let magnitude = clock_to_micros(&offset_clock)?;
    let is_negative = chars.get(sign_index) == Some(&'-');
    let signed = if is_negative {
        0_i64.checked_sub(magnitude)?
    } else {
        magnitude
    };
    Some((clock, Some(signed)))
}

// Mirrors _parse_hh_mm_ss_ff: HH[:?MM[:?SS[{.,}f[f]]]], a colon used for every pair or none of them, also used for an offset's own HH[:MM[:SS[.f]]], which carries no range check of its own.
fn parse_clock(chars: &[char]) -> Option<ClockFields> {
    let mut fields = [0_u32; 3];
    let mut position = 0;
    let mut has_separator = false;
    for (component_index, field) in fields.iter_mut().enumerate() {
        let (value, after_component) = take_digits(chars, position, 2)?;
        *field = value;
        position = after_component;
        let next_char = chars.get(position).copied();
        if component_index == 0 {
            has_separator = next_char == Some(':');
        }
        if next_char.is_none() || component_index >= 2 {
            break;
        }
        if has_separator && next_char != Some(':') {
            return None;
        }
        position = position.saturating_add(usize::from(has_separator));
    }
    let micro = if chars.get(position).is_some() {
        parse_fraction(chars, position)?
    } else {
        0
    };
    Some(ClockFields {
        hour: fields[0],
        minute: fields[1],
        second: fields[2],
        micro,
    })
}

// A fraction keeps its first six digits and drops the rest; an empty or non-digit fraction fails.
fn parse_fraction(chars: &[char], position: usize) -> Option<u32> {
    if !matches!(chars.get(position), Some('.' | ',')) {
        return None;
    }
    let digits_start = position.saturating_add(1);
    let digits = chars.get(digits_start..).unwrap_or(&[]);
    if digits.is_empty() || !digits.iter().all(char::is_ascii_digit) {
        return None;
    }
    let take = digits.len().min(FRACTION_WIDTH);
    let kept = digits.get(..take)?;
    let value: u32 = kept.iter().collect::<String>().parse().ok()?;
    let scale = *FRACTION_SCALE.get(take)?;
    value.checked_mul(scale)
}

fn clock_to_micros(clock: &ClockFields) -> Option<i64> {
    let hour_seconds = i64::from(clock.hour).checked_mul(SECONDS_PER_HOUR)?;
    let minute_seconds = i64::from(clock.minute).checked_mul(SECONDS_PER_MINUTE)?;
    let seconds = hour_seconds
        .checked_add(minute_seconds)?
        .checked_add(i64::from(clock.second))?;
    let micros = seconds.checked_mul(MICROS_PER_SECOND)?;
    micros.checked_add(i64::from(clock.micro))
}

fn build_instant(
    date: NaiveDate,
    clock: &ClockFields,
    offset_micros: Option<i64>,
) -> Option<DateTime<Utc>> {
    let time = NaiveTime::from_hms_micro_opt(clock.hour, clock.minute, clock.second, clock.micro)?;
    let naive = NaiveDateTime::new(date, time);
    let shifted = naive.checked_sub_signed(TimeDelta::microseconds(offset_micros.unwrap_or(0)))?;
    Some(shifted.and_utc())
}
