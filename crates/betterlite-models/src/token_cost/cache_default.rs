use rust_decimal::Decimal;

use super::error::TokenCostError;
use crate::off_peak::schema::{OffPeakBlock, RateField};
use crate::price_fields::decimal_lexeme::parse_decimal_lexeme;
use crate::tiered_pricing::Scalar;
use crate::token_rates::ModelInfo;

const FIREWORKS_CACHE_READ_RATIO: Decimal = Decimal::from_parts(5, 0, 0, false, 1);

// Mirrors cache_pricing.py's _as_rate: a bool is deliberately excluded even though it is an int subtype in Python.
fn as_rate(value: Option<&Scalar>) -> Option<Decimal> {
    match value {
        Some(Scalar::Number(number)) => parse_decimal_lexeme(&number.to_string()).ok(),
        Some(Scalar::Text(text)) => parse_decimal_lexeme(text.trim()).ok(),
        Some(Scalar::Bool(_) | Scalar::Null | Scalar::Array(_) | Scalar::Object(_)) | None => None,
    }
}

fn rate_field_as_rate(value: Option<&RateField>) -> Option<Decimal> {
    match value {
        Some(RateField::Number(number)) => parse_decimal_lexeme(&number.to_string()).ok(),
        Some(RateField::Text(text)) => parse_decimal_lexeme(text.trim()).ok(),
        Some(RateField::Bool(_) | RateField::Other(_)) | None => None,
    }
}

// BTreeMap::get can't tell a null off_peak_pricing key from an absent one, so the caller passes that check directly.
pub fn provider_cache_read_default(
    model_info: &ModelInfo,
    custom_llm_provider: Option<&str>,
    off_peak: Option<&OffPeakBlock>,
    off_peak_cache_read_present: bool,
) -> Result<(ModelInfo, Option<OffPeakBlock>), TokenCostError> {
    if custom_llm_provider == Some("fireworks_ai") {
        with_default_cache_read_rate(model_info, off_peak, off_peak_cache_read_present)
    } else {
        Ok((model_info.clone(), off_peak.cloned()))
    }
}

// An explicit nonnull cache price wins; otherwise half the valid non-boolean input rate covers the model and, when its own key is absent, the off-peak block too.
pub fn with_default_cache_read_rate(
    model_info: &ModelInfo,
    off_peak: Option<&OffPeakBlock>,
    off_peak_cache_read_present: bool,
) -> Result<(ModelInfo, Option<OffPeakBlock>), TokenCostError> {
    let cache_read_present = !matches!(
        model_info.get("cache_read_input_token_cost"),
        None | Some(Scalar::Null)
    );
    let Some(input_rate) =
        as_rate(model_info.get("input_cost_per_token")).filter(|_| !cache_read_present)
    else {
        return Ok((model_info.clone(), off_peak.cloned()));
    };
    let cache_read_rate = input_rate
        .checked_mul(FIREWORKS_CACHE_READ_RATIO)
        .ok_or(TokenCostError::Overflow)?;

    let mut updated = model_info.clone();
    updated.insert(
        "cache_read_input_token_cost".to_owned(),
        Scalar::Text(cache_read_rate.normalize().to_string()),
    );
    let updated_off_peak = off_peak
        .map(|block| with_off_peak_default(block, off_peak_cache_read_present, cache_read_rate))
        .transpose()?;
    Ok((updated, updated_off_peak))
}

fn with_off_peak_default(
    off_peak: &OffPeakBlock,
    cache_read_key_present: bool,
    cache_read_rate: Decimal,
) -> Result<OffPeakBlock, TokenCostError> {
    if cache_read_key_present {
        return Ok(off_peak.clone());
    }
    let off_peak_input_rate = rate_field_as_rate(off_peak.input_cost_per_token.as_ref());
    let rate = match off_peak_input_rate {
        Some(rate) => rate
            .checked_mul(FIREWORKS_CACHE_READ_RATIO)
            .ok_or(TokenCostError::Overflow)?,
        None => cache_read_rate,
    };
    let mut updated = off_peak.clone();
    updated.cache_read_input_token_cost = Some(RateField::Text(rate.normalize().to_string()));
    Ok(updated)
}

mod tests;
