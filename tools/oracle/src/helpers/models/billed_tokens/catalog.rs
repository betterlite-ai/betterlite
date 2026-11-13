use betterlite_models::billed_tokens::catalog::{default_pricing, map_billed_rates};
use betterlite_models::billed_tokens::savings::model_caching_savings;
use betterlite_models::billed_tokens::{
    BilledTokenError, BilledTokenRequest, breakdown, project_billed_token_counts,
};
use betterlite_models::price_lookup::PricedModel;
use jiff::tz::TimeZoneDatabase;
use serde_json::{Value, json};

use super::input::{Input, Operation};

pub(super) fn run(input: &Input) -> Result<Value, BilledTokenError> {
    let priced = PricedModel {
        record: input.model_info.prices.clone(),
        tiered_pricing: input.tiered_pricing.clone(),
        off_peak_pricing: input.off_peak_pricing.clone(),
        off_peak_cache_read_present: input
            .off_peak_pricing
            .as_ref()
            .is_some_and(|block| block.cache_read_input_token_cost.is_some()),
    };
    let request = BilledTokenRequest {
        model: "oracle",
        provider: input.provider.as_deref(),
        pricing: Some(&priced),
        service_tier: input.service_tier.as_deref(),
        residency: input.residency.as_deref(),
        vertex_location: input.vertex_location.as_deref(),
        inference_geo: input.inference_geo.as_deref(),
        provider_specific_entry: input.model_info.provider_specific_entry.as_ref(),
        moment: input.moment,
        prompt_tokens: input.totals.prompt_tokens,
    };
    price(input, &request, &priced)
}

fn price(
    input: &Input,
    request: &BilledTokenRequest<'_>,
    priced: &PricedModel,
) -> Result<Value, BilledTokenError> {
    let database = TimeZoneDatabase::from_env();
    let savings = model_caching_savings(request, priced, input.usage, &database)?;
    if matches!(input.operation, Operation::Savings) {
        return Ok(json!(savings.normalize().to_string()));
    }
    let defaulted = default_pricing(priced, request.provider)?;
    let rates = map_billed_rates(request, &defaulted, &database)?;
    let result = breakdown(project_billed_token_counts(input.usage), Some(rates))?;
    Ok(
        json!({"breakdown": super::breakdown_value(result), "savings": savings.normalize().to_string()}),
    )
}
