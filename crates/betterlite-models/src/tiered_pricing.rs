use crate::price_fields::decimal_lexeme::{DecimalLexemeError, parse_decimal_lexeme};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde::de::IgnoredAny;

/// A loosely typed JSON scalar.
///
/// The source's tolerant scalar coercion has to inspect a cost field's runtime
/// shape, not assume it is already a number.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Scalar {
    Bool(bool),
    Number(serde_json::Number),
    Text(String),
    Array(Vec<IgnoredAny>),
    Object(IgnoredAny),
}

/// A whole-request price tier. Field identity is the catalog's wire names, read
/// through named struct fields rather than a runtime string-keyed lookup.
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
    pub cache_creation_input_token_cost: Option<Scalar>,
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

/// Coerce a per-token cost into a [`Decimal`].
///
/// A syntactically invalid numeric string becomes zero, matching the source's
/// `except ValueError: return 0.0`; an out-of-range or unrepresentable number
/// is a typed error instead.
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

/// Read a tier's rate, preferring a present field over the fallback.
///
/// A present primary wins even when it is explicit zero. The caller resolves
/// which named field is primary and which is the fallback, so this never
/// interpolates a cost-field name itself.
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

/// Select the whole-request tier for `input_tokens`.
///
/// Tiers are stable-sorted by each tier's first range endpoint (zero when the
/// tier has no range) so sort ties keep their original order. A malformed
/// range, including one that is present but empty, errors here during the
/// sort rather than being filtered out beforehand. Only two-element ranges are
/// retained; a request matches the first retained tier with
/// `lower < input_tokens <= upper`, or else falls back to the last retained
/// tier, covering both an above-every-range request and a gap between two
/// ranges.
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
