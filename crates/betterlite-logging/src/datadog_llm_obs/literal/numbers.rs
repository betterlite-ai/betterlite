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

// ast.literal_eval allows one leading sign and a complex's real or imaginary part; JSON carries neither.
pub(super) fn number(parser: &mut Parser<'_>) -> Option<Json> {
    let negative = match parser.bytes.get(parser.position).copied()? {
        b'-' => {
            parser.position = parser.position.saturating_add(1);
            true
        }
        b'+' => {
            parser.position = parser.position.saturating_add(1);
            false
        }
        _ => false,
    };
    parser.skip(parser.depth > 0);
    let numeric = unsigned_number(parser)?;
    if let Some(b'j' | b'J') = parser.bytes.get(parser.position).copied() {
        parser.position = parser.position.saturating_add(1);
        return Some(Json::Text(PLACEHOLDER_LITERAL.to_owned()));
    }
    if matches!(
        parser.bytes.get(parser.position).copied(),
        Some(b'+' | b'-')
    ) {
        return imaginary_tail(parser);
    }
    match numeric {
        Numeric::Float(value) => {
            let sign = if negative { -1.0 } else { 1.0 };
            Some(
                Number::from_f64(value.copysign(sign))
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

// The imaginary part of a complex literal, which turns the whole number into the placeholder.
fn imaginary_tail(parser: &mut Parser<'_>) -> Option<Json> {
    parser.position = parser.position.saturating_add(1);
    parser.skip(parser.depth > 0);
    unsigned_number(parser)?;
    if !matches!(
        parser.bytes.get(parser.position).copied(),
        Some(b'j' | b'J')
    ) {
        return None;
    }
    parser.position = parser.position.saturating_add(1);
    Some(Json::Text(PLACEHOLDER_LITERAL.to_owned()))
}

fn leading_dot(parser: &mut Parser<'_>) -> Option<Numeric> {
    parser.position = parser.position.saturating_add(1);
    let fraction = digits(parser, 10)?;
    if fraction.is_empty() {
        return None;
    }
    let mut literal = format!("0.{fraction}");
    exponent(parser, &mut literal)?;
    literal.parse::<f64>().ok().map(Numeric::Float)
}

fn unsigned_number(parser: &mut Parser<'_>) -> Option<Numeric> {
    if parser.bytes.get(parser.position).copied() == Some(b'.') {
        return leading_dot(parser);
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
        parser.position = parser.position.saturating_add(2);
        if parser.bytes.get(parser.position).copied() == Some(b'_') {
            parser.position = parser.position.saturating_add(1);
        }
        let base_digits = digits(parser, radix)?;
        if base_digits.is_empty() {
            return None;
        }
        return Some(
            base_integer(&base_digits, radix).map_or(Numeric::Placeholder, Numeric::Integer),
        );
    }
    let integer = digits(parser, 10)?;
    if integer.is_empty() {
        return None;
    }
    let (literal, is_float) = decimal_literal(parser, &integer)?;
    if !is_float {
        return decimal_integer(&integer);
    }
    literal.parse::<f64>().ok().map(Numeric::Float)
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

// The exponent of a float literal, appended in the form f64 reads.
fn exponent(parser: &mut Parser<'_>, literal: &mut String) -> Option<bool> {
    if !matches!(
        parser.bytes.get(parser.position).copied(),
        Some(b'e' | b'E')
    ) {
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

// A provider repr writes integers in decimal, so a base-prefixed integer fits a u128 or is refused.
fn base_integer(digits: &str, radix: u32) -> Option<String> {
    u128::from_str_radix(digits, radix)
        .ok()
        .map(|value| value.to_string())
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
