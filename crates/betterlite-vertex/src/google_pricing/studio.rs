use betterlite_models::token_cost::GenericCostResult;

use super::delegate::{GenericTokenPricing, GenericTokenRequest};
use super::error::GooglePricingError;
use super::usage::GoogleUsage;

// Google AI Studio bills as provider gemini and never passes a vertex location
pub fn cost_per_token<P>(
    pricing: &P,
    model: &str,
    usage: &GoogleUsage,
    service_tier: Option<&str>,
) -> Result<GenericCostResult, GooglePricingError>
where
    P: GenericTokenPricing,
{
    pricing
        .generic_cost_per_token(&GenericTokenRequest {
            model,
            custom_llm_provider: "gemini",
            usage: &usage.tokens,
            service_tier,
            vertex_location: None,
        })
        .map_err(GooglePricingError::from)
}
