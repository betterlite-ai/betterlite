#![cfg(test)]

use rstest::rstest;

use crate::cache_aware_routing::AnthropicCacheRouting;
use crate::cache_aware_routing::deployment::PredictionDeployment;
use crate::cache_aware_routing::deployment::WarmRequest;
use crate::cache_aware_routing::tests::support::BODY;
use crate::cache_aware_routing::tests::support::CALLER;
use crate::cache_aware_routing::tests::support::DEPLOYMENT_ID;
use crate::cache_aware_routing::tests::support::FakeCost;
use crate::cache_aware_routing::tests::support::FakeMinimum;
use crate::cache_aware_routing::tests::support::MODEL;
use crate::cache_aware_routing::tests::support::MULTI_BODY;
use crate::cache_aware_routing::tests::support::MemoryCache;
use crate::cache_aware_routing::tests::support::PARAMS;
use crate::cache_aware_routing::tests::support::Resolution;
use crate::cache_aware_routing::tests::support::json;
use crate::cache_aware_routing::tests::support::model_info;
use crate::cache_aware_routing::tests::support::observation_json;
use crate::cache_aware_routing::tests::support::observation_key;
use crate::cache_aware_routing::tests::support::params;
use crate::cache_aware_routing::tests::support::prediction_services;
use crate::cache_aware_routing::tests::support::scope;
use crate::cache_aware_routing::tests::support::settings;

const NOW: f64 = 1_700_000_000.0;
const FRESH_UNTIL: f64 = 1_700_000_060.0;
const EXPIRED_AT: f64 = 1_699_999_999.0;

fn routing(body: &str) -> AnthropicCacheRouting {
    AnthropicCacheRouting::from_body(&json(body)).expect("a predictable body")
}

fn seed(cache: &MemoryCache, fingerprint: &str, cached_tokens: i64, expires_at: f64) {
    let key = observation_key(&scope(DEPLOYMENT_ID, MODEL), fingerprint);
    cache.seed(
        &key,
        observation_json(fingerprint, cached_tokens, NOW, expires_at),
    );
}

async fn warm(body: &str, cache: &MemoryCache, params_json: &str, now: f64) -> bool {
    let litellm_params = params(params_json);
    let info = model_info();
    let deployment = PredictionDeployment::read(&litellm_params, &info);
    let resolution = Resolution::new();
    let context = resolution.context();
    let settings = settings("https://api.anthropic.com");
    let cost = FakeCost::new(None);
    let minimum = FakeMinimum { tokens: 0 };
    let services = prediction_services(&settings, &context, cache, &cost, &minimum);
    let request = WarmRequest {
        deployment,
        caller_key_hash: CALLER,
        now,
    };
    routing(body).is_warm(&request, &services).await
}

#[tokio::test]
async fn a_fresh_observation_with_the_same_fingerprint_is_warm() {
    let cache = MemoryCache::new();
    let prefix = routing(BODY).prefix;
    seed(&cache, &prefix.fingerprint, 60, FRESH_UNTIL);
    assert!(warm(BODY, &cache, PARAMS, NOW).await);
}

#[tokio::test]
async fn an_observation_expiring_exactly_now_is_not_warm() {
    let cache = MemoryCache::new();
    let prefix = routing(BODY).prefix;
    seed(&cache, &prefix.fingerprint, 60, NOW);
    assert!(!warm(BODY, &cache, PARAMS, NOW).await);
}

#[tokio::test]
async fn an_expired_observation_is_not_warm() {
    let cache = MemoryCache::new();
    let prefix = routing(BODY).prefix;
    seed(&cache, &prefix.fingerprint, 60, EXPIRED_AT);
    assert!(!warm(BODY, &cache, PARAMS, NOW).await);
}

#[tokio::test]
async fn an_empty_cache_is_not_warm() {
    assert!(!warm(BODY, &MemoryCache::new(), PARAMS, NOW).await);
}

#[tokio::test]
async fn a_fresh_observation_for_another_prefix_is_not_warm() {
    let cache = MemoryCache::new();
    let prefix = routing(MULTI_BODY).prefix;
    assert!(prefix.fingerprints.len() > 1);
    seed(&cache, &prefix.fingerprints[1], 60, FRESH_UNTIL);
    assert!(!warm(MULTI_BODY, &cache, PARAMS, NOW).await);
}

#[rstest]
#[case::extra_option(
    r#"{"model":"anthropic/claude-x","api_key":"sk-ant-api03-x","organization":"org"}"#
)]
#[case::non_public_base(
    r#"{"model":"anthropic/claude-x","api_key":"sk-ant-api03-x","api_base":"https://proxy.test"}"#
)]
#[tokio::test]
async fn an_unpredictable_deployment_is_not_warm(#[case] params_json: &str) {
    let cache = MemoryCache::new();
    let prefix = routing(BODY).prefix;
    seed(&cache, &prefix.fingerprint, 60, FRESH_UNTIL);
    assert!(!warm(BODY, &cache, params_json, NOW).await);
}
