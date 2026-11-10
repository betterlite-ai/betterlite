use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

use crate::price_fields::decimal_lexeme::parse_decimal_lexeme;
use crate::tiered_pricing::Scalar;

pub(super) fn ptu_count(value: &Scalar) -> Option<u32> {
    let parsed = match value {
        Scalar::Number(number) => count_number(&number.to_string())?,
        Scalar::Text(text) => count_text(text)?,
        Scalar::Bool(_) | Scalar::Null | Scalar::Array(_) | Scalar::Object(_) => return None,
    };
    let count = u32::try_from(parsed).ok()?;
    (count > 0 && count <= super::MAX_PTU_COUNT).then_some(count)
}

pub(super) fn ptu_rate(value: &Scalar) -> Option<Decimal> {
    match value {
        Scalar::Number(number) => parse_decimal_lexeme(&number.to_string()).ok(),
        Scalar::Text(text) => rate_text(text),
        Scalar::Bool(_) | Scalar::Null | Scalar::Array(_) | Scalar::Object(_) => None,
    }
}

// int() truncates a float toward zero, so 2.9 becomes 2 while a value past Decimal's range fails like OverflowError.
fn count_number(lexeme: &str) -> Option<i128> {
    if lexeme.contains(['.', 'e', 'E']) {
        return parse_decimal_lexeme(lexeme).ok()?.trunc().to_i128();
    }
    lexeme.parse().ok()
}

fn count_text(text: &str) -> Option<i128> {
    let trimmed = text.trim();
    let negative = trimmed.starts_with('-');
    let digits = trimmed.strip_prefix(['-', '+']).unwrap_or(trimmed);
    if !digit_run(digits) {
        return None;
    }
    let magnitude = digits.replace('_', "").parse::<i128>().ok()?;
    if negative {
        magnitude.checked_neg()
    } else {
        Some(magnitude)
    }
}

// float() accepts a decimal literal with PEP 515 underscores, and an inf or nan spelling fails the finite bound after.
fn rate_text(text: &str) -> Option<Decimal> {
    let trimmed = text.trim();
    if !float_literal(trimmed) {
        return None;
    }
    parse_decimal_lexeme(&trimmed.replace('_', "")).ok()
}

pub(super) fn float_literal(text: &str) -> bool {
    let body = text.strip_prefix(['+', '-']).unwrap_or(text);
    let (mantissa, exponent) = match body.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (mantissa, Some(exponent)),
        None => (body, None),
    };
    let (whole, fraction) = match mantissa.split_once('.') {
        Some((whole, fraction)) => (whole, fraction),
        None => (mantissa, ""),
    };
    let has_digit = !whole.is_empty() || !fraction.is_empty();
    has_digit
        && (whole.is_empty() || digit_run(whole))
        && (fraction.is_empty() || digit_run(fraction))
        && exponent
            .is_none_or(|digits| digit_run(digits.strip_prefix(['+', '-']).unwrap_or(digits)))
}

fn digit_run(text: &str) -> bool {
    let mut previous_underscore = true;
    let mut saw_digit = false;
    for byte in text.bytes() {
        match byte {
            b'_' if previous_underscore => return false,
            b'_' => previous_underscore = true,
            b'0'..=b'9' => {
                saw_digit = true;
                previous_underscore = false;
            }
            _ => return false,
        }
    }
    saw_digit && !previous_underscore
}
