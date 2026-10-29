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
#[serde(default)]
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

// A syntactically invalid numeric string becomes zero, matching the source's `except ValueError: return 0.0`.
pub fn coerce_cost_per_token(value: Option<&Scalar>) -> Result<Decimal, TieredPricingError> {
    match value {
        None => Ok(Decimal::ZERO),
        Some(Scalar::Bool(flag)) => Ok(Decimal::from(u8::from(*flag))),
        Some(Scalar::Number(number)) => Ok(parse_decimal_lexeme(&number.to_string())?),
        Some(Scalar::Text(text)) => match parse_decimal_lexeme(text) {
            Err(DecimalLexemeError::Decimal(rust_decimal::Error::ErrorString(_))) => {
                Ok(Decimal::ZERO)
            }
            other => Ok(other?),
        },
        Some(Scalar::Array(_)) => Err(TieredPricingError::UnsupportedRateShape("array")),
        Some(Scalar::Object(_)) => Err(TieredPricingError::UnsupportedRateShape("object")),
    }
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

fn range_bounds(tier: &Tier) -> Option<(Decimal, Decimal)> {
    let [
        Scalar::Number(lower_endpoint),
        Scalar::Number(upper_endpoint),
    ] = tier.range.as_deref()?
    else {
        return None;
    };
    let lower_bound = parse_decimal_lexeme(&lower_endpoint.to_string()).ok()?;
    let upper_bound = parse_decimal_lexeme(&upper_endpoint.to_string()).ok()?;
    Some((lower_bound, upper_bound))
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

    let valid: Vec<(Decimal, Decimal, &Tier)> = keyed
        .into_iter()
        .filter_map(|(_, tier)| range_bounds(tier).map(|(lower, upper)| (lower, upper, tier)))
        .collect();
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
