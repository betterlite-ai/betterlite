use std::str::FromStr as _;

use betterlite_models::decisions::Json;
use serde_json::Number;

use super::{PLACEHOLDER_LITERAL, Parser};

// Python refuses to stringify an integer past this many digits, so a bigger literal cannot reach the intake.
const MAX_INTEGER_DIGITS: usize = 4300;

pub(super) enum Numeric {
    Integer(String),
    Float(f64),
    Placeholder,
}

pub(super) fn number(parser: &mut Parser<'_>) -> Option<Json> {
    let ellipsis = parser.position.saturating_add(3);
    if parser.bytes.get(parser.position..ellipsis) == Some(b"...".as_slice()) {
        parser.position = ellipsis;
        return Some(Json::Text(PLACEHOLDER_LITERAL.to_owned()));
    }
    let negative = parser.bytes.get(parser.position).copied()? == b'-';
    if negative || parser.bytes.get(parser.position).copied() == Some(b'+') {
        parser.position = parser.position.saturating_add(1);
    }
    parser.skip(parser.depth > 0);
    let numeric = unsigned_number(parser)?;
    if let Some(b'j' | b'J') = parser.bytes.get(parser.position).copied() {
        parser.position = parser.position.saturating_add(1);
        return Some(Json::Text(PLACEHOLDER_LITERAL.to_owned()));
    }
    let sign_at = parser.position;
    parser.skip(parser.depth > 0);
    let byte = parser.bytes.get(parser.position).copied();
    if matches!(byte, Some(b'+' | b'-')) {
        parser.position = parser.position.saturating_add(1);
        parser.skip(parser.depth > 0);
        unsigned_number(parser)?;
        let unit = parser.bytes.get(parser.position).copied();
        if !matches!(unit, Some(b'j' | b'J')) {
            return None;
        }
        parser.position = parser.position.saturating_add(1);
        return Some(Json::Text(PLACEHOLDER_LITERAL.to_owned()));
    }
    parser.position = sign_at;
    signed_value(numeric, negative)
}

fn signed_value(numeric: Numeric, negative: bool) -> Option<Json> {
    match numeric {
        Numeric::Float(value) => {
            let factor = if negative { -1.0 } else { 1.0 };
            Some(
                Number::from_f64(value.copysign(factor))
                    .map_or_else(|| Json::Text(PLACEHOLDER_LITERAL.to_owned()), Json::Number),
            )
        }
        Numeric::Integer(raw) => {
            let prefix = if negative && raw != "0" { "-" } else { "" };
            Number::from_str(&format!("{prefix}{raw}"))
                .ok()
                .map(Json::Number)
        }
        Numeric::Placeholder => Some(Json::Text(PLACEHOLDER_LITERAL.to_owned())),
    }
}

fn unsigned_number(parser: &mut Parser<'_>) -> Option<Numeric> {
    if parser.bytes.get(parser.position).copied() == Some(b'.') {
        parser.position = parser.position.saturating_add(1);
        let fraction = digits(parser, 10)?;
        if fraction.is_empty() {
            return None;
        }
        let mut literal = format!("0.{fraction}");
        exponent(parser, &mut literal)?;
        return literal.parse::<f64>().ok().map(Numeric::Float);
    }
    if parser.bytes.get(parser.position).copied() == Some(b'0')
        && let Some(radix) = parser
            .bytes
            .get(parser.position.saturating_add(1))
            .copied()
            .and_then(|byte| match byte {
                b'x' | b'X' => Some(16),
                b'o' | b'O' => Some(8),
                b'b' | b'B' => Some(2),
                _ => None,
            })
    {
        return base_number(parser, radix);
    }
    let integer = digits(parser, 10)?;
    if integer.is_empty() {
        return None;
    }
    let (literal, is_float) = decimal_literal(parser, &integer)?;
    if is_float {
        return literal.parse::<f64>().ok().map(Numeric::Float);
    }
    // An imaginary literal is a float, so its digit run may carry the leading zeros an int refuses.
    if matches!(
        parser.bytes.get(parser.position).copied(),
        Some(b'j' | b'J')
    ) {
        return Some(Numeric::Placeholder);
    }
    decimal_integer(&integer)
}

fn base_number(parser: &mut Parser<'_>, radix: u32) -> Option<Numeric> {
    parser.position = parser.position.saturating_add(2);
    if parser.bytes.get(parser.position).copied() == Some(b'_') {
        parser.position = parser.position.saturating_add(1);
    }
    let digits = digits(parser, radix)?;
    let unit = parser.bytes.get(parser.position).copied();
    if digits.is_empty() || matches!(unit, Some(b'j' | b'J')) {
        return None;
    }
    Some(
        u128::from_str_radix(&digits, radix)
            .ok()
            .map_or(Numeric::Placeholder, |value| {
                Numeric::Integer(value.to_string())
            }),
    )
}

fn decimal_literal(parser: &mut Parser<'_>, integer: &str) -> Option<(String, bool)> {
    let mut literal = integer.to_owned();
    let mut is_float = false;
    if parser.bytes.get(parser.position).copied() == Some(b'.') {
        parser.position = parser.position.saturating_add(1);
        is_float = true;
        literal.push('.');
        let fraction = digits(parser, 10)?;
        literal.push_str(&fraction);
        if fraction.is_empty() {
            literal.push('0');
        }
    }
    if exponent(parser, &mut literal)? {
        is_float = true;
    }
    Some((literal, is_float))
}

fn exponent(parser: &mut Parser<'_>, literal: &mut String) -> Option<bool> {
    let byte = parser.bytes.get(parser.position).copied();
    if !matches!(byte, Some(b'e' | b'E')) {
        return Some(false);
    }
    parser.position = parser.position.saturating_add(1);
    literal.push('e');
    if let Some(sign @ (b'+' | b'-')) = parser.bytes.get(parser.position).copied() {
        literal.push(char::from(sign));
        parser.position = parser.position.saturating_add(1);
    }
    let exponent_digits = digits(parser, 10)?;
    if exponent_digits.is_empty() {
        return None;
    }
    literal.push_str(&exponent_digits);
    Some(true)
}

fn digits(parser: &mut Parser<'_>, radix: u32) -> Option<String> {
    let mut digits = String::new();
    while let Some(byte) = parser.bytes.get(parser.position).copied() {
        if !char::from(byte).is_digit(radix) {
            break;
        }
        digits.push(char::from(byte));
        parser.position = parser.position.saturating_add(1);
        if parser.bytes.get(parser.position).copied() != Some(b'_') {
            continue;
        }
        let following = parser
            .bytes
            .get(parser.position.saturating_add(1))
            .copied()
            .is_some_and(|next| char::from(next).is_digit(radix));
        if !following {
            return None;
        }
        parser.position = parser.position.saturating_add(1);
    }
    Some(digits)
}

fn decimal_integer(digits: &str) -> Option<Numeric> {
    let trimmed = digits.trim_start_matches('0');
    if trimmed.is_empty() {
        return Some(Numeric::Integer("0".to_owned()));
    }
    if trimmed.len() != digits.len() {
        return None;
    }
    if digits.len() > MAX_INTEGER_DIGITS {
        return Some(Numeric::Placeholder);
    }
    Some(Numeric::Integer(digits.to_owned()))
}
