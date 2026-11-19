#![cfg(test)]

use rust_decimal::Decimal;

use crate::cache_aware_routing::AnthropicCacheRouting;
use crate::cache_aware_routing::tests::support::DEPLOYMENT_ID;
use crate::cache_aware_routing::tests::support::FakeCost;
use crate::cache_aware_routing::tests::support::MODEL;
use crate::cache_aware_routing::tests::support::billing_time;
use crate::cache_aware_routing::types::CacheCostScenario;
use crate::cache_aware_routing::types::CachePredictionArm;
use crate::cache_aware_routing::types::CacheState;
use crate::cache_aware_routing::types::CacheTokenBuckets;

const PRICE: Decimal = Decimal::from_parts(5, 0, 0, false, 2);

fn tokens() -> CacheTokenBuckets {
    CacheTokenBuckets {
        uncached_input_tokens: 40,
        cache_read_input_tokens: 60,
        cache_creation_5m_input_tokens: 0,
        cache_creation_1h_input_tokens: 0,
    }
}

fn arm(estimate: Option<CacheCostScenario>, model: Option<&str>) -> CachePredictionArm {
    CachePredictionArm {
        deployment_id: DEPLOYMENT_ID.to_owned(),
        model: model.map(str::to_owned),
        cache_state: CacheState::Unknown,
        reason: None,
        estimate,
        cold: None,
        warm: None,
        evidence: None,
        token_count_source: None,
    }
}

fn scenario() -> CacheCostScenario {
    CacheCostScenario {
        tokens: tokens(),
        input_cost: Decimal::ONE,
    }
}

#[test]
fn cost_prices_the_estimate_with_the_arm_identity() {
    let service = FakeCost::new(Some(PRICE));
    let arm = arm(Some(scenario()), Some(MODEL));
    let cost = AnthropicCacheRouting::cost(&service, &arm, 7, billing_time());
    assert_eq!(cost, Some(PRICE));
    let calls = service.prices();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, MODEL);
    assert_eq!(calls[0].deployment_id, DEPLOYMENT_ID);
    assert_eq!(calls[0].tokens, tokens());
    assert_eq!(calls[0].completion_tokens, 7);
    assert_eq!(calls[0].moment, billing_time());
}

#[test]
fn cost_without_an_estimate_is_none() {
    let service = FakeCost::new(Some(PRICE));
    let arm = arm(None, Some(MODEL));
    assert_eq!(
        AnthropicCacheRouting::cost(&service, &arm, 7, billing_time()),
        None
    );
    assert!(service.prices().is_empty());
}

#[test]
fn cost_prices_an_arm_without_a_model_name_as_the_empty_model() {
    let service = FakeCost::new(Some(PRICE));
    let arm = arm(Some(scenario()), None);
    assert_eq!(
        AnthropicCacheRouting::cost(&service, &arm, 0, billing_time()),
        Some(PRICE)
    );
    assert_eq!(service.prices()[0].model, "");
}

#[test]
fn cost_reports_the_price_service_failure() {
    let service = FakeCost::new(None);
    let arm = arm(Some(scenario()), Some(MODEL));
    assert_eq!(
        AnthropicCacheRouting::cost(&service, &arm, 7, billing_time()),
        None
    );
}
