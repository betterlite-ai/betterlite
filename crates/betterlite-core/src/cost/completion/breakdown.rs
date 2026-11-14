use betterlite_models::billed_tokens::catalog::catalog_billed_token_rates;
use betterlite_models::billed_tokens::custom::{custom_billed_token_rates, custom_breakdown};
use betterlite_models::billed_tokens::{
    BilledTokenError, BilledTokenRequest, TokenTypeCostBreakdown, breakdown,
};
use betterlite_models::cost_usage::billing_counters::billing_counters;
use betterlite_models::cost_usage::error::CostUsageError;
use betterlite_models::price_lookup::PriceLookup;
use betterlite_models::token_cost::error::TokenCostError;
use betterlite_models::token_cost::prompt_overlap::prompt_tokens_details;
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;

use crate::cost::call_price::CallPriceInput;
use crate::cost::call_price::error::CallPriceError;
use crate::cost::call_price::usage::usage_view;

#[derive(Debug, thiserror::Error)]
pub enum BreakdownError {
    #[error(transparent)]
    Usage(#[from] CallPriceError),
    #[error(transparent)]
    Prompt(#[from] TokenCostError),
    #[error(transparent)]
    Counters(#[from] CostUsageError),
    #[error(transparent)]
    Rates(#[from] BilledTokenError),
}

// What get_token_type_cost_breakdown reads besides the usage: the identity and the tier, residency and location uplifts.
pub struct BreakdownRequest<'request> {
    pub model: &'request str,
    pub provider: Option<&'request str>,
    pub moment: DateTime<Utc>,
}

// Ports get_token_type_cost_breakdown: flat custom rates when the call carries them, else the catalog rates the totals used.
pub fn token_type_breakdown(
    request: &BreakdownRequest<'_>,
    input: &CallPriceInput<'_>,
    lookup: &dyn PriceLookup,
    database: &TimeZoneDatabase,
) -> Result<TokenTypeCostBreakdown, BreakdownError> {
    let view = usage_view(input)?;
    let details = prompt_tokens_details(&view.cost, &view.extras)?;
    let counts = billing_counters(&view.cost, Some(&details))?;
    if let Some(custom) = input.custom_cost_per_token {
        let rates = custom_billed_token_rates(
            custom.input_cost_per_token,
            custom.output_cost_per_token,
            custom.cache_read_input_token_cost,
            custom.cache_creation_cost,
        );
        return Ok(custom_breakdown(counts, rates)?);
    }
    // inference_geo and provider_specific_entry are never set by completion_cost, so they stay absent.
    let billed = BilledTokenRequest {
        model: request.model,
        provider: request.provider,
        pricing: None,
        service_tier: input.service_tier,
        residency: input.data_residency,
        vertex_location: input.vertex_location,
        inference_geo: None,
        provider_specific_entry: None,
        moment: request.moment,
        prompt_tokens: view.cost.prompt_tokens,
    };
    let rates = catalog_billed_token_rates(&billed, lookup, database)?;
    Ok(breakdown(counts, rates)?)
}
