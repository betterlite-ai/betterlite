use betterlite_models::tiered_pricing::{
    Scalar, Tier, TieredPricingError, select_tier_for_input, tier_rate,
};
use serde::Deserialize;

pub(crate) struct Op;

// A closed enum of the tier's cost field identities, so an unmodeled key fails input parsing by name.
#[derive(Deserialize)]
pub(crate) enum CostKey {
    #[serde(rename = "input_cost_per_token")]
    InputToken,
    #[serde(rename = "output_cost_per_token")]
    OutputToken,
    #[serde(rename = "output_cost_per_reasoning_token")]
    OutputReasoningToken,
    #[serde(rename = "cache_read_input_token_cost")]
    CacheReadToken,
    #[serde(rename = "cache_creation_input_token_cost")]
    CacheCreationToken,
    #[serde(rename = "cache_creation_input_token_cost_above_1hr")]
    CacheCreationAbove1hr,
    #[serde(rename = "input_cost_per_query")]
    InputCostPerQuery,
}

#[derive(Deserialize)]
pub(crate) struct Input {
    tiers: Vec<Tier>,
    tokens: i64,
    cost_key: CostKey,
    fallback_cost_key: Option<CostKey>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let selected = select_tier_for_input(&input.tiers, input.tokens)
            .map_err(|error| pricing_error(&error))?;
        let Some(tier) = selected else {
            return Ok(serde_json::json!({ "selected_range": null, "rate": null }));
        };
        let primary = scalar_for(tier, &input.cost_key);
        let fallback = input
            .fallback_cost_key
            .as_ref()
            .and_then(|key| scalar_for(tier, key));
        let rate = tier_rate(primary, fallback).map_err(|error| pricing_error(&error))?;
        Ok(serde_json::json!({
            "selected_range": range_json(tier),
            "rate": rate.normalize().to_string(),
        }))
    }
}

const fn scalar_for<'tier>(tier: &'tier Tier, key: &CostKey) -> Option<&'tier Scalar> {
    match key {
        CostKey::InputToken => tier.input_token.as_ref(),
        CostKey::OutputToken => tier.output_token.as_ref(),
        CostKey::OutputReasoningToken => tier.output_reasoning_token.as_ref(),
        CostKey::CacheReadToken => tier.cache_read_input_token_cost.as_ref(),
        CostKey::CacheCreationToken => tier.cache_creation_token_cost.as_ref(),
        CostKey::CacheCreationAbove1hr => tier.cache_creation_above_1hr_cost.as_ref(),
        CostKey::InputCostPerQuery => tier.input_cost_per_query.as_ref(),
    }
}

fn range_json(tier: &Tier) -> Option<serde_json::Value> {
    let [Scalar::Number(lower), Scalar::Number(upper)] = tier.range.as_deref()? else {
        return None;
    };
    Some(serde_json::json!([lower, upper]))
}

fn pricing_error(error: &TieredPricingError) -> Box<crate::helpers::HelperError> {
    let message = error.to_string();
    Box::new(crate::helpers::HelperError {
        class: "TieredPricingError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

mod tests;
