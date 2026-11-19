use betterlite_models::billed_tokens::BilledTokenRates;
use betterlite_models::price_lookup::PriceLookup;
use betterlite_models::tiered_pricing::Scalar;
use betterlite_provider::pricing::grounding::ProviderFormulas;

use super::deployments::ResolvedCostModel;
use super::lines::CostLines;
use crate::cost_service::CostService;
use crate::key_fields::values::Amount;
use crate::proxy_request_contracts::cost_estimate::{CostEstimateRequest, CostEstimateResponse};

// The provider the response names: the backend's own declaration first, then the deployment's.
pub(super) fn reported_provider(
    service: &CostService<ProviderFormulas>,
    resolved: &ResolvedCostModel,
) -> Option<String> {
    let record = service
        .lookup
        .effective_model_info(&resolved.model, resolved.provider.as_deref());
    if let Some(Scalar::Text(provider)) = record
        .as_ref()
        .and_then(|found| found.get("litellm_provider"))
    {
        return Some(provider.clone());
    }
    resolved.provider.clone()
}

// The wire response: the request echoed, its lines scaled per period, and the billed rates.
#[rustfmt::skip]
pub(super) fn response_for(
    request: &CostEstimateRequest,
    lines: &CostLines,
    rates: Option<BilledTokenRates>,
    provider: Option<String>,
) -> CostEstimateResponse {
    let daily = lines.times(request.num_requests_per_day);
    let monthly = lines.times(request.num_requests_per_month);
    CostEstimateResponse {
        model: request.model.clone(), input_tokens: request.input_tokens, output_tokens: request.output_tokens,
        cache_read_input_tokens: request.cache_read_input_tokens, cache_creation_input_tokens: request.cache_creation_input_tokens,
        reasoning_tokens: request.reasoning_tokens, num_requests_per_day: request.num_requests_per_day,
        num_requests_per_month: request.num_requests_per_month,
        cost_per_request: Amount(lines.total), input_cost_per_request: Amount(lines.input),
        output_cost_per_request: Amount(lines.output), margin_cost_per_request: Amount(lines.margin),
        cache_read_cost_per_request: Amount(lines.cache_read), cache_creation_cost: Amount(lines.cache_creation),
        reasoning_cost_per_request: Amount(lines.reasoning),
        daily_cost: daily.as_ref().map(|found| Amount(found.total)), daily_input_cost: daily.as_ref().map(|found| Amount(found.input)),
        daily_output_cost: daily.as_ref().map(|found| Amount(found.output)), daily_margin_cost: daily.as_ref().map(|found| Amount(found.margin)),
        daily_cache_read_cost: daily.as_ref().map(|found| Amount(found.cache_read)),
        daily_cache_creation_cost: daily.as_ref().map(|found| Amount(found.cache_creation)),
        daily_reasoning_cost: daily.as_ref().map(|found| Amount(found.reasoning)),
        monthly_cost: monthly.as_ref().map(|found| Amount(found.total)), monthly_input_cost: monthly.as_ref().map(|found| Amount(found.input)),
        monthly_output_cost: monthly.as_ref().map(|found| Amount(found.output)), monthly_margin_cost: monthly.as_ref().map(|found| Amount(found.margin)),
        monthly_cache_read_cost: monthly.as_ref().map(|found| Amount(found.cache_read)),
        monthly_cache_creation_cost: monthly.as_ref().map(|found| Amount(found.cache_creation)),
        monthly_reasoning_cost: monthly.as_ref().map(|found| Amount(found.reasoning)),
        input_cost_per_token: rates.as_ref().map(|found| Amount(found.input_cost_per_token)),
        output_cost_per_token: rates.as_ref().map(|found| Amount(found.output_cost_per_token)),
        cache_read_input_token_cost: rates.as_ref().map(|found| Amount(found.cache_read_input_token_cost)),
        cache_creation_token_cost: rates.as_ref().map(|found| Amount(found.cache_write)),
        output_reasoning_token_cost: rates.as_ref().map(|found| Amount(found.reasoning)),
        provider,
    }
}
