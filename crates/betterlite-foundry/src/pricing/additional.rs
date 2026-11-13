use std::collections::BTreeMap;

use betterlite_context::chat_config_selection::foundry::select_config;
use betterlite_context::chat_config_selection::tags::ChatConfigTag;
use betterlite_models::price_lookup::PriceLookup;
use rust_decimal::Decimal;

use super::AZURE_AI_PROVIDER;
use super::error::PricingError;
use super::fee::router_flat_cost;

pub const ROUTER_FLAT_COST_LABEL: &str = "Azure Model Router Flat Cost";

// The hook reports a fee only when it is positive, unlike the direct calculation it wraps.
pub fn router_additional_costs(
    model: &str,
    prompt_tokens: i64,
    lookup: &dyn PriceLookup,
) -> Result<Option<BTreeMap<String, Decimal>>, PricingError> {
    let flat_cost = router_flat_cost(model, prompt_tokens, lookup)?;
    if flat_cost <= Decimal::ZERO {
        return Ok(None);
    }
    Ok(Some(BTreeMap::from([(
        ROUTER_FLAT_COST_LABEL.to_owned(),
        flat_cost,
    )])))
}

// Only the azure_ai model-router config supports this hook; the shared dispatcher never fails.
pub fn additional_costs(
    model: &str,
    custom_llm_provider: Option<&str>,
    prompt_tokens: i64,
    _completion_tokens: i64,
    lookup: &dyn PriceLookup,
) -> Option<BTreeMap<String, Decimal>> {
    if custom_llm_provider != Some(AZURE_AI_PROVIDER) {
        return None;
    }
    if select_config(model) != ChatConfigTag::AzureModelRouter {
        return None;
    }
    match router_additional_costs(model, prompt_tokens, lookup) {
        Ok(costs) => costs,
        Err(error) => {
            tracing::debug!(model = model, error = %error, "Error calculating additional costs");
            None
        }
    }
}
