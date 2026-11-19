use crate::cache_aware_routing::deployment::PredictionDeployment;
use crate::cache_aware_routing::services::PredictionServices;
use crate::cache_aware_routing::services::ScenarioRequest;
use crate::cache_aware_routing::types::CachePredictionArm;
use crate::cache_aware_routing::types::CacheState;
use crate::cache_aware_routing::types::CacheTokenBuckets;
use crate::cache_aware_routing::types::TokenCountSource;

pub(crate) const BELOW_MINIMUM: &str = "below_cache_minimum";

// The disabled arm prices the whole prompt as uncached and repeats it for all three scenarios.
pub(crate) async fn disabled_arm(
    deployment: &PredictionDeployment<'_>,
    model: &str,
    total: i64,
    services: &PredictionServices<'_>,
) -> Option<CachePredictionArm> {
    let tokens = CacheTokenBuckets {
        uncached_input_tokens: total,
        cache_read_input_tokens: 0,
        cache_creation_5m_input_tokens: 0,
        cache_creation_1h_input_tokens: 0,
    };
    let request = ScenarioRequest {
        model,
        deployment_id: deployment.deployment_id,
        tokens: &tokens,
    };
    let scenario = services.cost.scenario(&request, services.billing_time)?;
    Some(CachePredictionArm {
        deployment_id: deployment.deployment_id.to_owned(),
        model: Some(model.to_owned()),
        cache_state: CacheState::Disabled,
        reason: Some(BELOW_MINIMUM.to_owned()),
        estimate: Some(scenario.clone()),
        cold: Some(scenario.clone()),
        warm: Some(scenario),
        evidence: None,
        token_count_source: Some(TokenCountSource::AnthropicCountTokens),
    })
}

mod tests;
