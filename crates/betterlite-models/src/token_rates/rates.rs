use rust_decimal::Decimal;

use super::{ModelInfo, TokenRateError};
use crate::price_fields::decimal_lexeme::parse_decimal_lexeme;
use crate::tiered_pricing::Scalar;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ServiceTier {
    Ultrafast,
    Balanced,
    Priority,
    Auto,
    Flex,
    Fast,
}

impl ServiceTier {
    // Longest suffix first, so a tier-specific key is tried before a shorter sibling could match.
    pub(super) const ALL: [Self; 6] = [
        Self::Ultrafast,
        Self::Balanced,
        Self::Priority,
        Self::Auto,
        Self::Flex,
        Self::Fast,
    ];

    fn parse(tier: &str) -> Option<Self> {
        match tier.to_ascii_lowercase().as_str() {
            "ultrafast" => Some(Self::Ultrafast),
            "balanced" => Some(Self::Balanced),
            "priority" => Some(Self::Priority),
            "auto" => Some(Self::Auto),
            "flex" => Some(Self::Flex),
            "fast" => Some(Self::Fast),
            _ => None,
        }
    }

    pub(super) const fn suffix(self) -> &'static str {
        match self {
            Self::Ultrafast => "_ultrafast",
            Self::Balanced => "_balanced",
            Self::Priority => "_priority",
            Self::Auto => "_auto",
            Self::Flex => "_flex",
            Self::Fast => "_fast",
        }
    }

    const fn cost_key_suffix(self) -> Option<&'static str> {
        match self {
            Self::Flex => Some("_flex"),
            Self::Balanced => Some("_balanced"),
            Self::Priority | Self::Fast => Some("_priority"),
            Self::Ultrafast => Some("_ultrafast"),
            Self::Auto => None,
        }
    }
}

pub(super) fn service_tier_cost_key(base: &str, tier: Option<&str>) -> String {
    let Some(suffix) = tier
        .and_then(ServiceTier::parse)
        .and_then(ServiceTier::cost_key_suffix)
    else {
        return base.to_owned();
    };
    format!("{base}{suffix}")
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
        Scalar::Number(number) => {
            parse_decimal_lexeme(&number.to_string())
                .map(Some)
                .map_err(|source| TokenRateError::Rate {
                    key: key.to_owned(),
                    source,
                })
        }
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
        tracing::warn!(
            key,
            text,
            "malformed nonnull rate string uses the supplied default"
        );
        return Ok(default);
    }
    parse_decimal_lexeme(&text.replace('_', ""))
        .map(Some)
        .map_err(|source| TokenRateError::Rate {
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
            for suffix in ServiceTier::ALL.map(ServiceTier::suffix) {
                if key.contains(suffix) {
                    let base = key.replacen(suffix, "", 1);
                    return decimal_value(&base, model.get(&base), default);
                }
            }
            Ok(default)
        }
    }
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
