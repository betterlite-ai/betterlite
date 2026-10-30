use crate::price_fields::decimal_lexeme::{DecimalLexemeError, parse_decimal_lexeme};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde::de::IgnoredAny;
use std::collections::BTreeMap;

// The source's tolerant scalar coercion inspects a cost field's runtime shape, not just a number.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Scalar {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    Text(String),
    Array(Vec<IgnoredAny>),
    Object(BTreeMap<String, Option<IgnoredAny>>),
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Tier {
    #[serde(rename = "input_cost_per_token")]
    pub input_token: Option<Scalar>,
    #[serde(rename = "output_cost_per_token")]
    pub output_token: Option<Scalar>,
    #[serde(rename = "output_cost_per_reasoning_token")]
    pub output_reasoning_token: Option<Scalar>,
    pub cache_read_input_token_cost: Option<Scalar>,
    #[serde(rename = "cache_creation_input_token_cost")]
    pub cache_creation_token_cost: Option<Scalar>,
    #[serde(rename = "cache_creation_input_token_cost_above_1hr")]
    pub cache_creation_above_1hr_cost: Option<Scalar>,
    pub input_cost_per_query: Option<Scalar>,
    pub range: TierRange,
    pub max_results_range: Option<Vec<Scalar>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(untagged)]
pub enum TierRange {
    #[serde(skip)]
    #[default]
    Missing,
    Null(()),
    Values(Vec<Scalar>),
}

#[derive(Debug, thiserror::Error)]
pub enum TieredPricingError {
    #[error("a tier rate must be a number, string, boolean or null, not a JSON {0}")]
    UnsupportedRateShape(&'static str),
    #[error(transparent)]
    Rate(#[from] DecimalLexemeError),
    #[error("a tier's range is present but is not a two-element array of numbers")]
    MalformedRange,
}

// A string float() rejects becomes zero; one float() accepts but exact Decimal cannot is a typed error.
pub fn coerce_cost_per_token(value: Option<&Scalar>) -> Result<Decimal, TieredPricingError> {
    match value {
        None | Some(Scalar::Null) => Ok(Decimal::ZERO),
        Some(Scalar::Bool(flag)) => Ok(Decimal::from(u8::from(*flag))),
        Some(Scalar::Number(number)) => Ok(parse_decimal_lexeme(&number.to_string())?),
        Some(Scalar::Text(text)) => {
            let trimmed = text.trim();
            if is_python_float_literal(trimmed) {
                Ok(parse_decimal_lexeme(&trimmed.replace('_', ""))?)
            } else {
                Ok(Decimal::ZERO)
            }
        }
        Some(Scalar::Array(_)) => Err(TieredPricingError::UnsupportedRateShape("array")),
        Some(Scalar::Object(_)) => Err(TieredPricingError::UnsupportedRateShape("object")),
    }
}

// Mirrors the string grammar Python's float() accepts: a sign, then inf/nan or a decimal mantissa with an optional exponent.
fn is_python_float_literal(text: &str) -> bool {
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

// A present primary wins even when it is explicit zero; only a null/missing primary reads the fallback.
pub fn tier_rate(
    primary: Option<&Scalar>,
    fallback: Option<&Scalar>,
) -> Result<Decimal, TieredPricingError> {
    match primary {
        None => coerce_cost_per_token(fallback),
        present => coerce_cost_per_token(present),
    }
}

fn range_sort_key(tier: &Tier) -> Result<Decimal, TieredPricingError> {
    if matches!(tier.range, TierRange::Missing) {
        return Ok(Decimal::ZERO);
    }
    if matches!(tier.range, TierRange::Null(())) {
        return Err(TieredPricingError::MalformedRange);
    }
    let TierRange::Values(range) = &tier.range else {
        return Err(TieredPricingError::MalformedRange);
    };
    range
        .first()
        .map_or(Err(TieredPricingError::MalformedRange), range_endpoint)
}

fn range_endpoint(value: &Scalar) -> Result<Decimal, TieredPricingError> {
    if matches!(value, Scalar::Null) {
        return Err(TieredPricingError::MalformedRange);
    }
    if let Scalar::Bool(flag) = value {
        return Ok(Decimal::from(u8::from(*flag)));
    }
    let Scalar::Number(number) = value else {
        return Err(TieredPricingError::MalformedRange);
    };
    Ok(parse_decimal_lexeme(&number.to_string())?)
}

// A malformed range, including an empty one, errors during the sort rather than being filtered out first.
pub fn select_tier_for_input(
    tiers: &[Tier],
    input_tokens: i64,
) -> Result<Option<&Tier>, TieredPricingError> {
    if tiers.is_empty() || input_tokens <= 0 {
        return Ok(None);
    }
    let mut keyed = Vec::with_capacity(tiers.len());
    for tier in tiers {
        keyed.push((range_sort_key(tier)?, tier));
    }
    keyed.sort_by_key(|(key, _)| *key);

    let valid: Vec<_> = keyed
        .into_iter()
        .filter_map(|(lower, tier)| match &tier.range {
            TierRange::Values(range) if range.len() == 2 => Some((lower, tier)),
            TierRange::Missing | TierRange::Null(()) | TierRange::Values(_) => None,
        })
        .collect();
    let Some((_, last)) = valid.last() else {
        return Ok(None);
    };
    let input = Decimal::from(input_tokens);
    let mut first_match = None;
    for (lower, tier) in &valid {
        if *lower < input {
            let TierRange::Values(range) = &tier.range else {
                continue;
            };
            let [_, upper_scalar] = range.as_slice() else {
                continue;
            };
            let upper = range_endpoint(upper_scalar)?;
            if input <= upper {
                first_match = first_match.or(Some(*tier));
            }
        }
    }
    Ok(first_match.or(Some(*last)))
}

mod tests;
