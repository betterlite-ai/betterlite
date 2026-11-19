use betterlite_models::anthropic_pricing::ANTHROPIC_PROVIDER;
use betterlite_models::price_lookup::PriceRecord;
use betterlite_models::resolution::ProviderIdentity;

use crate::cost::context::IdentityRequest;
use crate::cost::context::ProviderInference;
use crate::cost::context::pricing::pricing_entry;
use crate::cost::prompt_cache_scenarios::PriceEnvironment;

// Ports _select_model_name_for_cost_calc with custom_pricing and the router id, then get_model_info and its registry key.
#[must_use]
pub fn selected_prices(
    environment: &PriceEnvironment<'_>,
    model: &str,
    deployment_id: &str,
) -> Option<(String, PriceRecord)> {
    pricing_entry(
        &IdentityRequest {
            model: Some(model),
            response: None,
            base_model: None,
            custom_pricing: Some(true),
            custom_llm_provider: Some(ANTHROPIC_PROVIDER),
            router_model_id: Some(deployment_id),
            region_name: None,
        },
        None,
        environment.lookup,
        &NamedProvider,
    )
    .ok()
    .flatten()
}

// Native prediction always names anthropic, so the shared billing path never has to infer a provider.
#[derive(Debug, thiserror::Error)]
#[error("a cache scenario always names its provider")]
struct NamedProvider;

impl ProviderInference for NamedProvider {
    type Failure = Self;

    fn inferred_provider(&self, _model: &str) -> Result<ProviderIdentity, Self> {
        Err(Self)
    }
}

mod tests;
