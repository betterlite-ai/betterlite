use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde_json::Number;

use crate::decisions::Json;

// cost_calculation.py get_anthropic_web_search_requests_from_response: only a mapping is eligible and the pinned Pydantic shape must validate.
#[must_use]
pub fn search_requests_from_response(response: &Json) -> Option<i64> {
    let Json::Object(fields) = response else {
        return None;
    };
    let Json::Object(usage) = fields.get("usage")? else {
        return None;
    };
    let Json::Object(server_tool_use) = usage.get("server_tool_use")? else {
        return None;
    };
    match server_tool_use.get("web_search_requests") {
        None | Some(Json::Null) => None,
        Some(value) => pydantic_integer(value),
    }
}

// Pydantic's lax `int | None` field: a Boolean or an integral number counts, a numeric string parses, and any other shape fails validation.
fn pydantic_integer(value: &Json) -> Option<i64> {
    match value {
        Json::Bool(flag) => Some(i64::from(*flag)),
        Json::Number(number) => number_as_integer(number),
        Json::Text(text) => text_as_integer(text),
        Json::Null | Json::List(_) | Json::Object(_) => None,
    }
}

fn number_as_integer(number: &Number) -> Option<i64> {
    if let Some(value) = number.as_i64() {
        return Some(value);
    }
    let literal = number.to_string();
    if !literal.contains(['.', 'e', 'E']) {
        return None;
    }
    float_as_integer(literal.parse().ok()?)
}

// Pydantic accepts an integral float strictly inside the signed 64-bit range and rejects both endpoints.
fn float_as_integer(value: f64) -> Option<i64> {
    if !value.is_finite() || value.fract() != 0.0_f64 {
        return None;
    }
    let decimal = Decimal::try_from(value).ok()?;
    if decimal <= Decimal::from(i64::MIN) || decimal >= Decimal::from(i64::MAX) {
        return None;
    }
    decimal.to_i64()
}

// Python's int(str) grammar as Pydantic narrows it: a sign, PEP-515 digits, and a fractional part that must be plain zeros.
fn text_as_integer(text: &str) -> Option<i64> {
    let trimmed = text.trim();
    let (negative, digits) = trimmed.strip_prefix('-').map_or_else(
        || (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
        |rest| (true, rest),
    );
    let (integer_part, fraction) = match digits.split_once('.') {
        Some((integer, fraction)) => (integer, Some(fraction)),
        None => (digits, None),
    };
    if !valid_underscored_digits(integer_part) {
        return None;
    }
    if let Some(fraction_digits) = fraction
        && (fraction_digits.is_empty()
            || !fraction_digits.bytes().all(|digit| digit.is_ascii_digit())
            || fraction_digits.bytes().any(|digit| digit != b'0'))
    {
        return None;
    }
    let cleaned: String = integer_part
        .chars()
        .filter(|character| *character != '_')
        .collect();
    let magnitude: i64 = cleaned.parse().ok()?;
    if negative {
        magnitude.checked_neg()
    } else {
        Some(magnitude)
    }
}

fn valid_underscored_digits(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.is_empty() || bytes.first() == Some(&b'_') {
        return false;
    }
    let mut previous_was_underscore = false;
    for &current in bytes {
        match current {
            b'_' if previous_was_underscore => return false,
            b'_' => previous_was_underscore = true,
            digit if digit.is_ascii_digit() => previous_was_underscore = false,
            _ => return false,
        }
    }
    !previous_was_underscore
}

mod tests;
