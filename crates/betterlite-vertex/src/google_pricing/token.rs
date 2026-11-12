use betterlite_models::catalog::CatalogEntry;
use betterlite_models::token_cost::GenericCostResult;
use rust_decimal::Decimal;

use super::delegate::{GenericTokenPricing, GenericTokenRequest};
use super::error::GooglePricingError;
use super::usage::GoogleUsage;

pub struct TokenRequest<'request> {
    pub model: &'request str,
    pub custom_llm_provider: &'request str,
    pub service_tier: Option<&'request str>,
    pub vertex_location: Option<&'request str>,
}

pub fn cost_per_token<P>(
    pricing: &P,
    record: &CatalogEntry,
    usage: &GoogleUsage,
    request: &TokenRequest<'_>,
) -> Result<GenericCostResult, GooglePricingError>
where
    P: GenericTokenPricing,
{
    let prices = &record.prices.values;
    if prices.input.input_token_over_128k.is_none()
        && prices.output.output_token_over_128k.is_none()
    {
        return pricing
            .generic_cost_per_token(&GenericTokenRequest {
                model: request.model,
                custom_llm_provider: request.custom_llm_provider,
                usage: &usage.tokens,
                service_tier: request.service_tier,
                vertex_location: request.vertex_location,
            })
            .map_err(GooglePricingError::from);
    }
    above_128k_costs(pricing, record, usage, request)
}

// the source enters this branch when either above-128k rate exists, then picks each side's rate on its own
fn above_128k_costs<P>(
    pricing: &P,
    record: &CatalogEntry,
    usage: &GoogleUsage,
    request: &TokenRequest<'_>,
) -> Result<GenericCostResult, GooglePricingError>
where
    P: GenericTokenPricing,
{
    let prices = &record.prices.values;
    let prompt_tokens = usage.tokens.prompt_tokens;
    let completion_tokens = usage.tokens.completion_tokens;
    let prompt_rate = selected_rate(
        prompt_tokens > 128_000,
        prices.input.input_token_over_128k,
        prices.input.input_token,
    );
    let completion_rate = selected_rate(
        completion_tokens > 128_000,
        prices.output.output_token_over_128k,
        prices.output.output_token,
    );
    let uplift = pricing.vertex_endpoint_uplift(record, request.vertex_location);
    Ok(GenericCostResult {
        prompt_cost: uplifted(prompt_tokens, prompt_rate, uplift, "prompt token")?,
        completion_cost: uplifted(
            completion_tokens,
            completion_rate,
            uplift,
            "completion token",
        )?,
    })
}

fn selected_rate(above: bool, above_rate: Option<Decimal>, base_rate: Option<Decimal>) -> Decimal {
    match (above, above_rate) {
        (true, Some(rate)) => rate,
        _ => base_rate.unwrap_or_default(),
    }
}

fn uplifted(
    tokens: i64,
    rate: Decimal,
    uplift: Decimal,
    field: &'static str,
) -> Result<Decimal, GooglePricingError> {
    Decimal::from(tokens)
        .checked_mul(rate)
        .and_then(|cost| cost.checked_mul(uplift))
        .ok_or(GooglePricingError::Overflow { field })
}
