use std::collections::BTreeMap;

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde::Deserialize;

use super::dynamic::MappingOrScalar;
use super::error::SavingsEvidenceError;
use crate::price_fields::decimal_lexeme::parse_decimal_lexeme;
use crate::tiered_pricing::Scalar;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct UsageObject {
    pub cache_read_input_tokens: Option<Scalar>,
    pub cache_creation_input_tokens: Option<Scalar>,
    pub prompt_tokens_details: Option<MappingOrScalar<PromptTokensDetailsRaw>>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(try_from = "BTreeMap<String, Scalar>")]
pub struct PromptTokensDetailsRaw {
    pub cached_tokens: Option<Scalar>,
    pub cache_write_tokens: Option<Scalar>,
    pub cache_creation_tokens: Option<Scalar>,
}

impl From<BTreeMap<String, Scalar>> for PromptTokensDetailsRaw {
    fn from(mut map: BTreeMap<String, Scalar>) -> Self {
        Self {
            cached_tokens: map.remove("cached_tokens"),
            cache_write_tokens: map.remove("cache_write_tokens"),
            cache_creation_tokens: map.remove("cache_creation_tokens"),
        }
    }
}

// Anthropic writes a top-level field; OpenAI-compatible providers nest it under prompt_tokens_details.
pub fn extract_cache_read_tokens(
    usage_object: Option<&UsageObject>,
) -> Result<i64, SavingsEvidenceError> {
    let Some(usage) = usage_object else {
        return Ok(0);
    };
    if let Some(count) = scalar_to_i64(
        usage.cache_read_input_tokens.as_ref(),
        Truthiness::NonzeroOnly,
    )? {
        return Ok(count);
    }
    let detail = usage
        .prompt_tokens_details
        .as_ref()
        .and_then(MappingOrScalar::as_mapping)
        .and_then(|detail| detail.cached_tokens.as_ref());
    Ok(scalar_to_i64(detail, Truthiness::IncludeZero)?.unwrap_or(0))
}

// A detail fallback here tries cache_write_tokens before cache_creation_tokens, the reverse of the typed split used elsewhere.
pub fn extract_cache_creation_tokens(
    usage_object: Option<&UsageObject>,
) -> Result<i64, SavingsEvidenceError> {
    let prompt_tokens_details = usage_object.and_then(|usage| usage.prompt_tokens_details.as_ref());
    let top_level = usage_object.and_then(|usage| usage.cache_creation_input_tokens.as_ref());
    let details = prompt_tokens_details.and_then(MappingOrScalar::as_mapping);
    let write_tokens = details.and_then(|detail| detail.cache_write_tokens.as_ref());
    let creation_tokens = details.and_then(|detail| detail.cache_creation_tokens.as_ref());
    for candidate in [top_level, write_tokens] {
        if let Some(count) = scalar_to_i64(candidate, Truthiness::NonzeroOnly)? {
            return Ok(count);
        }
    }
    Ok(scalar_to_i64(creation_tokens, Truthiness::IncludeZero)?.unwrap_or(0))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Truthiness {
    NonzeroOnly,
    IncludeZero,
}

// Truthiness is read from the decimal before truncation, so a fraction like 0.4 is nonzero even though int(0.4) is 0.
fn scalar_to_i64(
    value: Option<&Scalar>,
    truthiness: Truthiness,
) -> Result<Option<i64>, SavingsEvidenceError> {
    let decimal = match value {
        Some(Scalar::Bool(flag)) => Decimal::from(i64::from(*flag)),
        Some(Scalar::Number(number)) => parse_decimal_lexeme(&number.to_string())
            .map_err(|_source| SavingsEvidenceError::SavedTokensOverflow)?,
        _ => return Ok(None),
    };
    if matches!(truthiness, Truthiness::NonzeroOnly) && decimal.is_zero() {
        return Ok(None);
    }
    decimal
        .trunc()
        .to_i64()
        .map(Some)
        .ok_or(SavingsEvidenceError::SavedTokensOverflow)
}

mod tests;
