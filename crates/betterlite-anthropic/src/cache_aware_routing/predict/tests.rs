#![cfg(test)]

use rust_decimal::Decimal;

use crate::cache_aware_routing::AnthropicCacheRouting;
use crate::cache_aware_routing::deployment::ArmRequest;
use crate::cache_aware_routing::deployment::PredictionDeployment;
use crate::cache_aware_routing::tests::support::BODY;
use crate::cache_aware_routing::tests::support::CALLER;
use crate::cache_aware_routing::tests::support::DEPLOYMENT_ID;
use crate::cache_aware_routing::tests::support::FakeCost;
use crate::cache_aware_routing::tests::support::FakeMinimum;
use crate::cache_aware_routing::tests::support::MODEL;
use crate::cache_aware_routing::tests::support::MemoryCache;
use crate::cache_aware_routing::tests::support::PARAMS;
use crate::cache_aware_routing::tests::support::Resolution;
use crate::cache_aware_routing::tests::support::ScriptedCounter;
use crate::cache_aware_routing::tests::support::json;
use crate::cache_aware_routing::tests::support::params;
use crate::cache_aware_routing::tests::support::prediction_services;
use crate::cache_aware_routing::tests::support::settings;
use crate::cache_aware_routing::types::CachePredictionArm;
use crate::cache_aware_routing::types::CacheState;
use crate::cache_aware_routing::types::TokenCountSource;

const CHECKED_AT: f64 = 1_700_000_000.0;

async fn predict(
    params_json: &str,
    blocked: bool,
    replies: &[Option<i64>],
    cost: Option<Decimal>,
) -> (CachePredictionArm, ScriptedCounter) {
    let routing = AnthropicCacheRouting::from_body(&json(BODY)).expect("a predictable body");
    let litellm_params = params(params_json);
    let info = params(&format!(
        r#"{{"id":"{DEPLOYMENT_ID}","blocked":{blocked}}}"#
    ));
    let deployment = PredictionDeployment::read(&litellm_params, &info);
    let resolution = Resolution::new();
    let context = resolution.context();
    let settings = settings("https://api.anthropic.com");
    let cache = MemoryCache::new();
    let fake_cost = FakeCost::new(cost);
    let minimum = FakeMinimum { tokens: 0 };
    let services = prediction_services(&settings, &context, &cache, &fake_cost, &minimum);
    let counter = ScriptedCounter::new(replies);
    let request = ArmRequest {
        deployment,
        caller_key_hash: CALLER,
        checked_at: CHECKED_AT,
    };
    let arm = routing.predict(&request, &counter, &services).await;
    (arm, counter)
}

#[tokio::test]
async fn predict_prices_an_unobserved_prefix_through_the_native_counter() {
    let (arm, counter) = predict(PARAMS, false, &[Some(100), Some(40)], Some(Decimal::ONE)).await;
    assert_eq!(arm.deployment_id, DEPLOYMENT_ID);
    assert_eq!(arm.model.as_deref(), Some(MODEL));
    assert_eq!(arm.cache_state, CacheState::Unknown);
    assert_eq!(arm.reason.as_deref(), Some("no_compatible_observation"));
    assert_eq!(
        arm.token_count_source,
        Some(TokenCountSource::AnthropicCountTokens)
    );
    assert!(arm.evidence.is_none());
    let estimate = arm.estimate.expect("an estimate");
    assert_eq!(estimate.tokens.uncached_input_tokens, 60);
    assert_eq!(estimate.tokens.cache_creation_5m_input_tokens, 40);
    assert_eq!(estimate.tokens.cache_read_input_tokens, 0);
    assert!(arm.cold.is_some() && arm.warm.is_some());
    let asked = counter.asked();
    assert_eq!(asked.len(), 2);
    assert_eq!(asked[0], json(BODY));
}

#[tokio::test]
async fn predict_never_counts_a_blocked_deployment() {
    let (arm, counter) = predict(PARAMS, true, &[], Some(Decimal::ONE)).await;
    assert_eq!(arm.cache_state, CacheState::Unknown);
    assert_eq!(
        arm.reason.as_deref(),
        Some("unsupported_deployment_configuration")
    );
    assert_eq!(arm.model.as_deref(), Some("anthropic/claude-x"));
    assert!(arm.estimate.is_none());
    assert!(counter.asked().is_empty());
}
