use std::collections::BTreeMap;
use std::str::FromStr as _;

use chrono::{DateTime, NaiveDate, TimeZone as _, Utc};
use rust_decimal::Decimal;
use speedate::{Date, DateTime as SpeedateDateTime};

use crate::list_api::plan::types::{ListSpec, PAGE_PARAM, PAGE_SIZE_PARAM};
use crate::list_api::problem;
use crate::management_envelopes::ProblemDetail;

struct ParsedInteger {
    magnitude: u128,
    negative: bool,
    overflow: bool,
}

const DECIMAL_DIGIT_BLOCKS: &[u32] = &[
    0x0030, 0x0660, 0x06f0, 0x07c0, 0x0966, 0x09e6, 0x0a66, 0x0ae6, 0x0b66, 0x0be6, 0x0c66, 0x0ce6,
    0x0d66, 0x0de6, 0x0e50, 0x0ed0, 0x0f20, 0x1040, 0x1090, 0x17e0, 0x1810, 0x1946, 0x19d0, 0x1a80,
    0x1a90, 0x1b50, 0x1bb0, 0x1c40, 0x1c50, 0xa620, 0xa8d0, 0xa900, 0xa9d0, 0xa9f0, 0xaa50, 0xabf0,
    0xff10, 0x104a0, 0x10d30, 0x11066, 0x110f0, 0x11136, 0x111d0, 0x112f0, 0x11450, 0x114d0,
    0x115d8, 0x11650, 0x116c0, 0x11730, 0x118e0, 0x11950, 0x11c50, 0x11d50, 0x11da0, 0x11f50,
    0x16a60, 0x16ac0, 0x16b50, 0x1d7ce, 0x1d7d8, 0x1d7e2, 0x1d7ec, 0x1d7f6, 0x1e140, 0x1e2f0,
    0x1e4f0, 0x1e950, 0x1fbf0,
];

fn decimal_digit(character: char) -> Option<u32> {
    let codepoint = u32::from(character);
    DECIMAL_DIGIT_BLOCKS
        .iter()
        .find_map(|start| codepoint.checked_sub(*start).filter(|digit| *digit < 10))
}

fn signed_digits(raw: &str) -> (bool, &str) {
    let trimmed = raw.trim();
    let (negative, digits) = trimmed.strip_prefix('-').map_or_else(
        || (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
        |rest| (true, rest),
    );
    (negative, digits)
}

fn parse_magnitude(digits: &str) -> Option<(u128, bool)> {
    if digits.is_empty() {
        return None;
    }
    let mut magnitude = 0_u128;
    let mut overflow = false;
    let mut previous_was_digit = false;
    let mut characters = digits.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '_' {
            if !previous_was_digit
                || characters
                    .peek()
                    .and_then(|next| decimal_digit(*next))
                    .is_none()
            {
                return None;
            }
            previous_was_digit = false;
            continue;
        }
        let digit = u128::from(decimal_digit(character)?);
        if let Some(next_magnitude) = magnitude
            .checked_mul(10)
            .and_then(|value| value.checked_add(digit))
        {
            magnitude = next_magnitude;
        } else {
            overflow = true;
        }
        previous_was_digit = true;
    }
    previous_was_digit.then_some((magnitude, overflow))
}

fn parse_query_integer(raw: &str) -> Option<ParsedInteger> {
    let (negative, digits) = signed_digits(raw);
    let (magnitude, overflow) = parse_magnitude(digits)?;
    Some(ParsedInteger {
        magnitude,
        negative,
        overflow,
    })
}

fn positive_query_integer(name: &str, raw: &str) -> Result<ParsedInteger, ProblemDetail> {
    let parsed = parse_query_integer(raw)
        .ok_or_else(|| problem::invalid_query_parameter(format!("'{name}' must be an integer.")))?;
    if parsed.negative || parsed.magnitude == 0 {
        return Err(problem::invalid_query_parameter(format!(
            "'{name}' must be 1 or greater."
        )));
    }
    Ok(parsed)
}

pub(in crate::list_api::plan) fn parse_positive_int(
    name: &str,
    raw: &str,
) -> Result<u64, ProblemDetail> {
    let parsed = positive_query_integer(name, raw)?;
    if parsed.overflow {
        return Err(problem::invalid_query_parameter(format!(
            "'{name}' is too large."
        )));
    }
    u64::try_from(parsed.magnitude)
        .map_err(|_too_large| problem::invalid_query_parameter(format!("'{name}' is too large.")))
}

pub(in crate::list_api::plan) fn parse_page(
    params: &BTreeMap<String, String>,
) -> Result<u64, ProblemDetail> {
    params
        .get(PAGE_PARAM)
        .map_or(Ok(1), |raw| parse_positive_int(PAGE_PARAM, raw))
}

pub(in crate::list_api::plan) fn parse_page_size(
    spec: &ListSpec,
    params: &BTreeMap<String, String>,
) -> Result<u64, ProblemDetail> {
    match params.get(PAGE_SIZE_PARAM) {
        None => Ok(spec.default_page_size),
        Some(raw) => {
            let parsed = positive_query_integer(PAGE_SIZE_PARAM, raw)?;
            if parsed.overflow || parsed.magnitude > u128::from(spec.max_page_size) {
                return Ok(spec.max_page_size);
            }
            u64::try_from(parsed.magnitude).map_err(|_too_large| {
                problem::invalid_query_parameter(format!("'{PAGE_SIZE_PARAM}' is too large."))
            })
        }
    }
}

pub(super) fn parse_integer_filter(raw: &str) -> Option<i64> {
    let parsed = parse_query_integer(raw).filter(|parsed| !parsed.overflow)?;
    let signed = if parsed.negative {
        i128::try_from(parsed.magnitude).ok()?.checked_neg()?
    } else {
        i128::try_from(parsed.magnitude).ok()?
    };
    i64::try_from(signed).ok()
}

fn to_utc(parsed: &SpeedateDateTime) -> Option<DateTime<Utc>> {
    let nanoseconds = parsed.time.microsecond.checked_mul(1_000)?;
    Utc.timestamp_opt(parsed.timestamp_tz(), nanoseconds)
        .single()
}

fn parse_compact_date(raw: &str) -> Option<DateTime<Utc>> {
    let date = NaiveDate::parse_from_str(raw, "%Y%m%d").ok()?;
    let midnight = date.and_hms_opt(0, 0, 0)?;
    Some(DateTime::from_naive_utc_and_offset(midnight, Utc))
}

pub(super) fn parse_timestamp(raw: &str) -> Option<DateTime<Utc>> {
    if raw.len() == 8 && raw.bytes().all(|byte| byte.is_ascii_digit()) {
        return parse_compact_date(raw);
    }
    let parsed = SpeedateDateTime::parse_str(raw).or_else(|_| {
        let date = Date::parse_str(raw)?;
        SpeedateDateTime::parse_str(&format!("{date}T00:00:00"))
    });
    to_utc(&parsed.ok()?)
}

fn normalized_float(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let mut normalized = String::with_capacity(trimmed.len());
    let mut characters = trimmed.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '_' {
            let previous_is_digit = normalized
                .chars()
                .last()
                .is_some_and(|prior| prior.is_ascii_digit());
            let next_is_digit = characters.peek().is_some_and(char::is_ascii_digit);
            if !previous_is_digit || !next_is_digit {
                return None;
            }
        } else {
            normalized.push(character);
        }
    }
    let finite = normalized.parse::<f64>().ok()?.is_finite();
    finite.then_some(normalized)
}

pub(super) fn parse_decimal(raw: &str) -> Option<Decimal> {
    let normalized = normalized_float(raw)?;
    if normalized.contains(['e', 'E']) {
        Decimal::from_scientific(&normalized).ok()
    } else {
        Decimal::from_str(&normalized).ok()
    }
}
