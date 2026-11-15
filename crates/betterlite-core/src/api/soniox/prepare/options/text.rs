use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive as _, ToPrimitive as _};

pub(super) fn seconds(text: &str) -> Option<f64> {
    let trimmed = text.trim();
    if !underscores_join_digits(trimmed) {
        return None;
    }
    trimmed.replace('_', "").parse::<f64>().ok()
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

// int() turns a finite float past i64's range into an out-of-range int, which the caller clamps
pub(super) fn truncated_attempts(seconds: f64) -> i64 {
    let negative = seconds.is_sign_negative();
    let Some(number) = Decimal::from_f64(seconds) else {
        return saturate(negative);
    };
    number.to_i64().unwrap_or_else(|| saturate(negative))
}

pub(super) fn saturated_integer(literal: &str) -> i64 {
    let negative = literal.starts_with('-');
    let Ok(number) = literal.parse::<i128>() else {
        return saturate(negative);
    };
    i64::try_from(number).unwrap_or_else(|_| saturate(negative))
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
