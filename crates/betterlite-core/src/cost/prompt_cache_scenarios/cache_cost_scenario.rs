use betterlite_anthropic::cache_aware_routing::services::PriceRequest;
use betterlite_anthropic::cache_aware_routing::services::ScenarioRequest;
use betterlite_anthropic::cache_aware_routing::types::CacheCostScenario;
use chrono::DateTime;
use chrono::Utc;

use crate::cost::prompt_cache_scenarios::PriceEnvironment;
use crate::cost::prompt_cache_scenarios::price_cache_tokens::price_cache_tokens;

// Ports _scenario: one bucket set priced with no completion tokens, or nothing when the price is unknown.
#[must_use]
pub fn cache_cost_scenario(
    environment: &PriceEnvironment<'_>,
    moment: DateTime<Utc>,
    request: &ScenarioRequest<'_>,
) -> Option<CacheCostScenario> {
    let priced = PriceRequest {
        model: request.model,
        deployment_id: request.deployment_id,
        tokens: request.tokens,
        completion_tokens: 0,
    };
    let input_cost = price_cache_tokens(environment, moment, &priced)?;
    Some(CacheCostScenario {
        tokens: *request.tokens,
        input_cost,
    })
}

mod tests;
