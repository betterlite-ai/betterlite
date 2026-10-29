use crate::price_fields::decimal_lexeme::{DecimalLexemeError, parse_decimal_lexeme};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde::de::IgnoredAny;

// The source's tolerant scalar coercion inspects a cost field's runtime shape, not just a number.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Scalar {
    Bool(bool),
    Number(serde_json::Number),
    Text(String),
    Array(Vec<IgnoredAny>),
    Object(IgnoredAny),
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
    pub range: Option<Vec<Scalar>>,
    pub max_results_range: Option<Vec<Scalar>>,
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
        None => Ok(Decimal::ZERO),
        Some(Scalar::Bool(flag)) => Ok(Decimal::from(u8::from(*flag))),
        Some(Scalar::Number(number)) => Ok(parse_decimal_lexeme(&number.to_string())?),
        Some(Scalar::Text(text)) => {
            let trimmed = text.trim();
            if is_python_float_literal(trimmed) {
                Ok(parse_decimal_lexeme(trimmed)?)
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
        return !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
    };
    (!int_part.is_empty() || !frac_part.is_empty())
        && (int_part.is_empty() || int_part.bytes().all(|byte| byte.is_ascii_digit()))
        && (frac_part.is_empty() || frac_part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn is_signed_digits(text: &str) -> bool {
    let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
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
    match &tier.range {
        None => Ok(Decimal::ZERO),
        Some(range) => match range.first() {
            Some(Scalar::Number(lower)) => Ok(parse_decimal_lexeme(&lower.to_string())?),
            _ => Err(TieredPricingError::MalformedRange),
        },
    }
}

// A non-two-length range is filtered out; a two-length range with a bad endpoint is a typed error.
fn range_bounds(tier: &Tier) -> Result<Option<(Decimal, Decimal)>, TieredPricingError> {
    let Some([lower_scalar, upper_scalar]) = tier.range.as_deref() else {
        return Ok(None);
    };
    let (Scalar::Number(lower_number), Scalar::Number(upper_number)) = (lower_scalar, upper_scalar)
    else {
        return Err(TieredPricingError::MalformedRange);
    };
    let lower_bound = parse_decimal_lexeme(&lower_number.to_string())?;
    let upper_bound = parse_decimal_lexeme(&upper_number.to_string())?;
    Ok(Some((lower_bound, upper_bound)))
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

    let mut valid = Vec::with_capacity(keyed.len());
    for (_, tier) in keyed {
        if let Some((lower, upper)) = range_bounds(tier)? {
            valid.push((lower, upper, tier));
        }
    }
    let Some(&(_, _, last)) = valid.last() else {
        return Ok(None);
    };
    let input = Decimal::from(input_tokens);
    let selected = valid
        .iter()
        .find(|(lower, upper, _)| *lower < input && input <= *upper)
        .map_or(last, |&(_, _, tier)| tier);
    Ok(Some(selected))
}

mod tests;
