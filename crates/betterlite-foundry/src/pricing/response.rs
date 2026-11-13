use betterlite_models::cost_usage::CostUsage;
use betterlite_models::price_lookup::PriceLookup;
use betterlite_models::token_cost::error::TokenCostError;
use betterlite_models::token_cost::prompt_overlap::PromptExtras;
use betterlite_models::token_cost::{
    GenericCostRequest, GenericCostResult, generic_cost_per_token,
};
use chrono::{DateTime, Utc};
use jiff::tz::TimeZoneDatabase;
use rust_decimal::Decimal;

use super::AZURE_AI_PROVIDER;
use super::error::PricingError;
use super::fee::router_flat_cost;
use super::names::{classify_router_name, router_fee_name};

pub struct ResponseCostRequest<'request> {
    pub model: &'request str,
    pub usage: &'request CostUsage,
    pub prompt_extras: &'request PromptExtras,
    // the source signature carries this parameter and never reads it
    pub response_time_ms: Option<Decimal>,
    pub request_model: Option<&'request str>,
    pub service_tier: Option<&'request str>,
    pub moment: DateTime<Utc>,
}

pub fn cost_per_token(
    request: &ResponseCostRequest<'_>,
    lookup: &dyn PriceLookup,
    database: &TimeZoneDatabase,
) -> Result<GenericCostResult, PricingError> {
    let response = response_model_cost(request, lookup, database)?;
    let Some(fee_name) = router_fee_name(request.model, request.request_model) else {
        return Ok(response);
    };
    let fee = router_flat_cost(fee_name, request.usage.prompt_tokens, lookup)?;
    let prompt_cost = response
        .prompt_cost
        .checked_add(fee)
        .ok_or(PricingError::Overflow)?;
    Ok(GenericCostResult {
        prompt_cost,
        completion_cost: response.completion_cost,
    })
}

fn response_model_cost(
    request: &ResponseCostRequest<'_>,
    lookup: &dyn PriceLookup,
    database: &TimeZoneDatabase,
) -> Result<GenericCostResult, PricingError> {
    let generic_request = GenericCostRequest {
        model: request.model,
        custom_llm_provider: Some(AZURE_AI_PROVIDER),
        usage: request.usage,
        prompt_extras: request.prompt_extras,
        supplied_pricing: None,
        service_tier: request.service_tier,
        data_residency: None,
        vertex_location: None,
        moment: request.moment,
    };
    match generic_cost_per_token(&generic_request, lookup, database) {
        Ok(result) => Ok(result),
        Err(error) => router_pricing_fallback(request.model, error),
    }
}

// Only a router-shaped response model may be missing from the cost map; its fee still applies.
fn router_pricing_fallback(
    model: &str,
    error: TokenCostError,
) -> Result<GenericCostResult, PricingError> {
    if !classify_router_name(model).is_router_shaped() {
        return Err(PricingError::TokenCost(error));
    }
    tracing::debug!(
        model = model,
        error = %error,
        "Azure AI Model Router: model is not in the cost map, only the routing fee applies"
    );
    Ok(GenericCostResult {
        prompt_cost: Decimal::ZERO,
        completion_cost: Decimal::ZERO,
    })
}
