use std::collections::BTreeMap;
use std::str::FromStr as _;

use chrono::{DateTime, NaiveDate, TimeZone as _, Utc};
use rust_decimal::Decimal;
use speedate::{Date, DateTime as SpeedateDateTime};

use crate::list_api::plan::types::{ListSpec, PAGE_PARAM, PAGE_SIZE_PARAM};
use crate::list_api::problem;
use crate::management_envelopes::ProblemDetail;

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

fn parse_query_integer(raw: &str) -> Option<(bool, u128)> {
    let trimmed = raw.trim();
    let negative = trimmed.starts_with('-');
    let digits = trimmed.strip_prefix(['-', '+']).unwrap_or(trimmed);
    let mut magnitude = 0_u128;
    let mut previous_was_digit = false;
    for character in digits.chars() {
        if character == '_' && previous_was_digit {
            previous_was_digit = false;
        } else {
            let digit = u128::from(decimal_digit(character)?);
            // Saturation preserves positivity and permits capping before native conversion.
            magnitude = magnitude.saturating_mul(10).saturating_add(digit);
            previous_was_digit = true;
        }
    }
    previous_was_digit.then_some((negative, magnitude))
}

fn positive_query_integer(name: &str, raw: &str) -> Result<u128, ProblemDetail> {
    let (negative, magnitude) = parse_query_integer(raw)
        .ok_or_else(|| problem::invalid_query_parameter(format!("'{name}' must be an integer.")))?;
    if negative || magnitude == 0 {
        return Err(problem::invalid_query_parameter(format!(
            "'{name}' must be 1 or greater."
        )));
    }
    Ok(magnitude)
}

pub(in crate::list_api::plan) fn parse_positive_int(
    name: &str,
    raw: &str,
) -> Result<u64, ProblemDetail> {
    u64::try_from(positive_query_integer(name, raw)?)
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
            let capped =
                positive_query_integer(PAGE_SIZE_PARAM, raw)?.min(u128::from(spec.max_page_size));
            u64::try_from(capped).map_err(|_too_large| {
                problem::invalid_query_parameter(format!("'{PAGE_SIZE_PARAM}' is too large."))
            })
        }
    }
}

pub(super) fn parse_integer_filter(raw: &str) -> Option<i64> {
    let (negative, magnitude) = parse_query_integer(raw)?;
    let signed = if negative {
        i128::try_from(magnitude).ok()?.checked_neg()?
    } else {
        i128::try_from(magnitude).ok()?
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

fn normalized_decimal(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let mut normalized = String::with_capacity(trimmed.len());
    let mut characters = trimmed.chars().peekable();
    while let Some(character) = characters.next() {
        if character == '_' {
            let previous_is_digit = normalized.as_bytes().last().is_some_and(u8::is_ascii_digit);
            let next_is_digit = characters.peek().is_some_and(char::is_ascii_digit);
            if !previous_is_digit || !next_is_digit {
                return None;
            }
        } else if character.is_ascii_digit() || matches!(character, '+' | '-' | '.' | 'e' | 'E') {
            normalized.push(character);
        } else {
            return None;
        }
    }
    Some(normalized)
}

pub(super) fn parse_decimal(raw: &str) -> Option<Decimal> {
    let normalized = normalized_decimal(raw)?;
    if let Some((_, exponent)) = normalized.split_once(['e', 'E']) {
        let digits = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        Decimal::from_scientific(&normalized).ok()
    } else {
        Decimal::from_str(&normalized).ok()
    }
}
