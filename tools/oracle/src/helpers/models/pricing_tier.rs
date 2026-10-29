use betterlite_models::tiered_pricing::{Scalar, Tier, select_tier_for_input, tier_rate};
use serde::Deserialize;

pub(crate) struct Op;

#[derive(Deserialize)]
pub(crate) struct Input {
    tiers: Vec<Tier>,
    tokens: i64,
    cost_key: String,
    fallback_cost_key: Option<String>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let selected = select_tier_for_input(&input.tiers, input.tokens).map_err(pricing_error)?;
        let Some(tier) = selected else {
            return Ok(serde_json::json!({ "selected_range": null, "rate": null }));
        };
        let primary = scalar_for(tier, &input.cost_key);
        let fallback = input
            .fallback_cost_key
            .as_deref()
            .and_then(|key| scalar_for(tier, key));
        let rate = tier_rate(primary, fallback).map_err(pricing_error)?;
        Ok(serde_json::json!({
            "selected_range": range_json(tier),
            "rate": rate.normalize().to_string(),
        }))
    }
}

fn scalar_for<'tier>(tier: &'tier Tier, key: &str) -> Option<&'tier Scalar> {
    match key {
        "input_cost_per_token" => tier.input_token.as_ref(),
        "output_cost_per_token" => tier.output_token.as_ref(),
        "output_cost_per_reasoning_token" => tier.output_reasoning_token.as_ref(),
        "cache_read_input_token_cost" => tier.cache_read_input_token_cost.as_ref(),
        "cache_creation_input_token_cost" => tier.cache_creation_token_cost.as_ref(),
        "input_cost_per_query" => tier.input_cost_per_query.as_ref(),
        _ => None,
    }
}

fn range_json(tier: &Tier) -> Option<serde_json::Value> {
    let [Scalar::Number(lower), Scalar::Number(upper)] = tier.range.as_deref()? else {
        return None;
    };
    Some(serde_json::json!([lower, upper]))
}

fn pricing_error(error: impl std::fmt::Display) -> Box<crate::helpers::HelperError> {
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
