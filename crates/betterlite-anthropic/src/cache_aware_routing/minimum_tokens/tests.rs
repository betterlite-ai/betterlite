#![cfg(test)]

use rust_decimal::Decimal;

use super::BELOW_MINIMUM;
use super::disabled_arm;
use crate::cache_aware_routing::deployment::PredictionDeployment;
use crate::cache_aware_routing::tests::support::DEPLOYMENT_ID;
use crate::cache_aware_routing::tests::support::FakeCost;
use crate::cache_aware_routing::tests::support::FakeMinimum;
use crate::cache_aware_routing::tests::support::MODEL;
use crate::cache_aware_routing::tests::support::MemoryCache;
use crate::cache_aware_routing::tests::support::PARAMS;
use crate::cache_aware_routing::tests::support::Resolution;
use crate::cache_aware_routing::tests::support::billing_time;
use crate::cache_aware_routing::tests::support::model_info;
use crate::cache_aware_routing::tests::support::params;
use crate::cache_aware_routing::tests::support::prediction_services;
use crate::cache_aware_routing::tests::support::settings;
use crate::cache_aware_routing::types::CachePredictionArm;
use crate::cache_aware_routing::types::CacheState;
use crate::cache_aware_routing::types::TokenCountSource;

async fn disabled(cost: Option<Decimal>, total: i64) -> (Option<CachePredictionArm>, FakeCost) {
    let litellm_params = params(PARAMS);
    let info = model_info();
    let deployment = PredictionDeployment::read(&litellm_params, &info);
    let resolution = Resolution::new();
    let context = resolution.context();
    let settings = settings("https://api.anthropic.com");
    let cache = MemoryCache::new();
    let fake_cost = FakeCost::new(cost);
    let minimum = FakeMinimum { tokens: 0 };
    let services = prediction_services(&settings, &context, &cache, &fake_cost, &minimum);
    let arm = disabled_arm(&deployment, MODEL, total, &services).await;
    (arm, fake_cost)
}

#[tokio::test]
async fn disabled_arm_prices_the_whole_prompt_as_uncached() {
    let (arm, _cost) = disabled(Some(Decimal::ONE), 120).await;
    let prediction = arm.expect("a disabled arm");
    assert_eq!(prediction.cache_state, CacheState::Disabled);
    assert_eq!(prediction.reason.as_deref(), Some(BELOW_MINIMUM));
    assert_eq!(prediction.deployment_id, DEPLOYMENT_ID);
    assert_eq!(prediction.model.as_deref(), Some(MODEL));
    assert!(prediction.evidence.is_none());
    assert_eq!(
        prediction.token_count_source,
        Some(TokenCountSource::AnthropicCountTokens)
    );
    let estimate = prediction.estimate.clone().expect("an estimate");
    assert_eq!(estimate.tokens.uncached_input_tokens, 120);
    assert_eq!(estimate.tokens.total_tokens(), 120);
    assert_eq!(estimate.tokens.cache_read_input_tokens, 0);
    assert_eq!(prediction.cold, Some(estimate.clone()));
    assert_eq!(prediction.warm, Some(estimate));
}

#[tokio::test]
async fn disabled_arm_prices_the_deployment_identity_once() {
    let (arm, cost) = disabled(Some(Decimal::ONE), 120).await;
    assert!(arm.is_some());
    let calls = cost.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, MODEL);
    assert_eq!(calls[0].deployment_id, DEPLOYMENT_ID);
    assert_eq!(calls[0].moment, billing_time());
}

#[tokio::test]
async fn disabled_arm_without_a_price_is_none() {
    let (arm, cost) = disabled(None, 120).await;
    assert!(arm.is_none());
    assert_eq!(cost.calls().len(), 1);
}
