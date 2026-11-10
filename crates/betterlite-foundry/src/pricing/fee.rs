use betterlite_models::price_lookup::PriceLookup;
use betterlite_models::tiered_pricing::coerce_cost_per_token;
use rust_decimal::Decimal;

use super::AZURE_AI_PROVIDER;
use super::error::PricingError;
use super::names::{is_azure_model_router, router_fee_entry_name};

// A name that is not router-shaped charges nothing and never reaches the cost map.
pub fn router_flat_cost(
    model: &str,
    prompt_tokens: i64,
    lookup: &dyn PriceLookup,
) -> Result<Decimal, PricingError> {
    if !is_azure_model_router(model) {
        return Ok(Decimal::ZERO);
    }
    let entry = router_fee_entry_name(model);
    let record = lookup
        .effective_model_info(entry, Some(AZURE_AI_PROVIDER))
        .ok_or(PricingError::MissingFeeEntry { entry })?;
    let rate = coerce_cost_per_token(record.get("input_cost_per_token"))
        .map_err(|source| PricingError::FeeRate { entry, source })?;
    if rate <= Decimal::ZERO {
        return Ok(Decimal::ZERO);
    }
    Decimal::from(prompt_tokens)
        .checked_mul(rate)
        .ok_or(PricingError::Overflow)
}
