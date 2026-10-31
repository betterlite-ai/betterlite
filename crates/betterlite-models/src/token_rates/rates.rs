use rust_decimal::Decimal;

use super::{ModelInfo, TokenRateError};
use crate::price_fields::decimal_lexeme::parse_decimal_lexeme;
use crate::tiered_pricing::Scalar;

// Longest suffix first, so a tier-specific key is tried before a shorter sibling could match.
pub(super) const ALL_SERVICE_SUFFIXES: [&str; 6] = [
    "_ultrafast",
    "_balanced",
    "_priority",
    "_auto",
    "_flex",
    "_fast",
];

pub(super) fn service_tier_cost_key(base: &str, tier: Option<&str>) -> String {
    let Some(lowered) = tier.map(str::to_ascii_lowercase) else {
        return base.to_owned();
    };
    let suffix = match lowered.as_str() {
        "fast" | "priority" => "priority",
        "flex" => "flex",
        "balanced" => "balanced",
        "ultrafast" => "ultrafast",
        _ => return base.to_owned(),
    };
    format!("{base}_{suffix}")
}

pub(super) fn decimal_value(
    key: &str,
    value: Option<&Scalar>,
    default: Option<Decimal>,
) -> Result<Option<Decimal>, TokenRateError> {
    let Some(scalar) = value else {
        return Ok(default);
    };
    match scalar {
        Scalar::Bool(flag) => Ok(Some(Decimal::from(u8::from(*flag)))),
        Scalar::Number(number) => parse_rate(key, &number.to_string()).map(Some),
        Scalar::Text(text) => parse_text_rate(key, text.trim(), default),
        Scalar::Null | Scalar::Array(_) | Scalar::Object(_) => Ok(default),
    }
}

// A float() cannot hold (inf, nan, an overflowing exponent) is a typed error, not a default.
pub(super) fn parse_text_rate(
    key: &str,
    text: &str,
    default: Option<Decimal>,
) -> Result<Option<Decimal>, TokenRateError> {
    if !matches_float_literal_grammar(text) {
        return Ok(default);
    }
    parse_rate(key, &text.replace('_', "")).map(Some)
}

pub(super) fn parse_rate(key: &str, text: &str) -> Result<Decimal, TokenRateError> {
    let parsed = parse_decimal_lexeme(text);
    parsed.map_err(|source| TokenRateError::Rate {
        key: key.to_owned(),
        source,
    })
}

pub(super) fn cost_per_unit(
    model: &ModelInfo,
    key: &str,
    default: Option<Decimal>,
) -> Result<Option<Decimal>, TokenRateError> {
    match model.get(key) {
        Some(value @ (Scalar::Text(_) | Scalar::Number(_) | Scalar::Bool(_))) => {
            decimal_value(key, Some(value), default)
        }
        Some(Scalar::Array(_) | Scalar::Object(_)) => Ok(default),
        Some(Scalar::Null) | None => {
            // Only the first (longest) matching suffix anywhere in the key is tried, per the source's single `break`.
            for suffix in ALL_SERVICE_SUFFIXES {
                if key.contains(suffix) {
                    let base = key.replacen(suffix, "", 1);
                    return decimal_value(&base, model.get(&base), default);
                }
            }
            Ok(default)
        }
    }
}

pub(super) fn rate(
    model: &ModelInfo,
    key: &str,
    default: Decimal,
) -> Result<Decimal, TokenRateError> {
    let resolved = cost_per_unit(model, key, Some(default))?;
    Ok(resolved.unwrap_or(default))
}

pub(super) fn is_present(model: &ModelInfo, key: &str) -> bool {
    let found = model.get(key);
    !matches!(found, None | Some(Scalar::Null))
}

// Python float() grammar, duplicated from tiered_pricing's private check since it isn't pub.
pub(super) fn matches_float_literal_grammar(text: &str) -> bool {
    let body = text.strip_prefix(['+', '-']).unwrap_or(text);
    if body.eq_ignore_ascii_case("inf")
        || body.eq_ignore_ascii_case("infinity")
        || body.eq_ignore_ascii_case("nan")
    {
        return true;
    }
    let (mantissa, exponent) = match body.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (mantissa, Some(exponent)),
        None => (body, None),
    };
    is_decimal_mantissa(mantissa) && exponent.is_none_or(is_signed_digits)
}

fn is_decimal_mantissa(text: &str) -> bool {
    let Some((int_part, frac_part)) = text.split_once('.') else {
        return is_underscored_digits(text);
    };
    (!int_part.is_empty() || !frac_part.is_empty())
        && (int_part.is_empty() || is_underscored_digits(int_part))
        && (frac_part.is_empty() || is_underscored_digits(frac_part))
}

fn is_signed_digits(text: &str) -> bool {
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
    is_underscored_digits(digits)
}

// PEP 515: a digit run may carry single underscores between two digits, never leading, trailing or doubled.
fn is_underscored_digits(text: &str) -> bool {
    let bytes = text.as_bytes();
    let (Some(&first), Some(&last)) = (bytes.first(), bytes.last()) else {
        return false;
    };
    if first == b'_' || last == b'_' {
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
    true
}
