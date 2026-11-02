use rust_decimal::Decimal;

use super::error::DurationCostError;
use super::numeric::numeric_price;
use crate::price_lookup::{PriceLookup, PriceRecord};
use crate::tiered_pricing::Scalar;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum CostMetric {
    Character,
    Token,
}

// Picks by presence (docs/compatibility.md), not the source's truthiness, so an explicit 0 still selects its metric.
pub fn select_cost_metric_for_model(
    key: &str,
    record: &PriceRecord,
) -> Result<CostMetric, DurationCostError> {
    if !matches!(
        record.get("input_cost_per_character"),
        None | Some(Scalar::Null)
    ) {
        return Ok(CostMetric::Character);
    }
    if !matches!(
        record.get("input_cost_per_token"),
        None | Some(Scalar::Null)
    ) {
        return Ok(CostMetric::Token);
    }
    Err(DurationCostError::NoCharacterOrTokenPricing {
        key: key.to_owned(),
    })
}

pub struct CharacterCostRequest<'request> {
    pub model: &'request str,
    pub custom_llm_provider: &'request str,
    pub prompt_characters: Decimal,
    pub completion_characters: Decimal,
    pub custom_prompt_cost: Option<Decimal>,
    pub custom_completion_cost: Option<Decimal>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CharacterCostResult {
    pub prompt_cost: Option<Decimal>,
    pub completion_cost: Option<Decimal>,
}

pub fn generic_cost_per_character(
    request: &CharacterCostRequest<'_>,
    lookup: &dyn PriceLookup,
) -> Result<CharacterCostResult, DurationCostError> {
    let record = lookup
        .effective_model_info(request.model, Some(request.custom_llm_provider))
        .ok_or_else(|| DurationCostError::UnknownModel {
            model: request.model.to_owned(),
        })?;
    Ok(CharacterCostResult {
        prompt_cost: character_side_cost(
            &record,
            "input_cost_per_character",
            request.custom_prompt_cost,
            request.prompt_characters,
        ),
        completion_cost: character_side_cost(
            &record,
            "output_cost_per_character",
            request.custom_completion_cost,
            request.completion_characters,
        ),
    })
}

// Each side fails on its own: a missing or wrong-type price warns and returns None for that side only.
fn character_side_cost(
    record: &PriceRecord,
    key: &str,
    custom_cost: Option<Decimal>,
    characters: Decimal,
) -> Option<Decimal> {
    let rate = match custom_cost {
        Some(rate) => rate,
        None => match numeric_price(record, key) {
            Ok(Some(rate)) => rate,
            Ok(None) => {
                tracing::warn!(key, "model info does not have this character pricing");
                return None;
            }
            Err(error) => {
                tracing::warn!(key, %error, "could not resolve this character pricing");
                return None;
            }
        },
    };
    let cost = characters.checked_mul(rate);
    if cost.is_none() {
        tracing::warn!(key, "character cost overflowed Decimal arithmetic");
    }
    cost
}

mod tests;
