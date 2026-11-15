use rust_decimal::Decimal;
use rust_decimal::prelude::FromPrimitive as _;

// float() and int() refuse an integer literal past f64's range with OverflowError
const I64_CEILING: f64 = 9_223_372_036_854_775_808.0;
const I64_FLOOR: f64 = -9_223_372_036_854_775_808.0;

pub(super) fn seconds(text: &str) -> Option<f64> {
    let trimmed = text.trim();
    if !underscores_join_digits(trimmed) {
        return None;
    }
    let cleaned = trimmed.replace('_', "");
    numeric_text(&cleaned)?.parse::<f64>().ok()
}

// float() reads ".5" and "5." while Rust's number parser wants a digit on both sides
fn numeric_text(text: &str) -> Option<String> {
    let (sign, rest) = match text.as_bytes().first() {
        Some(b'+' | b'-') => text.split_at(1),
        _ => ("", text),
    };
    let (mantissa, exponent) = rest.split_at(rest.find(['e', 'E']).unwrap_or(rest.len()));
    let (whole, fraction) = mantissa.split_at(mantissa.find('.').unwrap_or(mantissa.len()));
    if whole.is_empty() && fraction.len() <= 1 {
        return None;
    }
    let whole = if whole.is_empty() { "0" } else { whole };
    let fraction = if fraction == "." { ".0" } else { fraction };
    Some(format!("{sign}{whole}{fraction}{exponent}"))
}

pub(super) fn attempts(text: &str) -> Option<i64> {
    let trimmed = text.trim();
    if !underscores_join_digits(trimmed) {
        return None;
    }
    let cleaned = trimmed.replace('_', "");
    let digits = cleaned
        .strip_prefix('+')
        .or_else(|| cleaned.strip_prefix('-'))
        .unwrap_or(cleaned.as_str());
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(saturated_integer(cleaned.as_str()))
}

pub(super) fn truncated_attempts(seconds: f64) -> Option<i64> {
    if seconds >= I64_CEILING {
        return Some(i64::MAX);
    }
    if seconds <= I64_FLOOR {
        return Some(i64::MIN);
    }
    Decimal::from_f64(seconds).and_then(|number| number.to_i64())
}

pub(super) fn saturated_integer(literal: &str) -> i64 {
    match literal.parse::<i128>() {
        Ok(number) => i64::try_from(number).unwrap_or(saturate(number.is_negative())),
        Err(_) => saturate(literal.starts_with('-')),
    }
}

// float() and int() take an underscore only when digits sit on both sides
fn underscores_join_digits(text: &str) -> bool {
    if text.starts_with('_') || text.ends_with('_') {
        return false;
    }
    text.as_bytes().windows(3).all(|window| match window {
        [before, b'_', after] => before.is_ascii_digit() && after.is_ascii_digit(),
        _ => true,
    })
}

const fn saturate(negative: bool) -> i64 {
    if negative { i64::MIN } else { i64::MAX }
}
