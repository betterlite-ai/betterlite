#![cfg(test)]

use std::sync::Arc;

use betterlite_redis::cache_primitives::protocol::RedisScalar;

use super::support::{
    FakeCache, FakeScript, FakeUsage, WINDOW_START, adjustments_with, guarded_operation,
    operation_tuples, script_handle, unguarded_operation,
};
use crate::limiter_records::{CacheCounterValue, ReservationAwareIncrement, ReservationBackend};
use crate::rate_limit_window_store::RedisCacheKind;
use crate::token_adjustments::router::apply_reservation_aware_tokens;
use crate::token_adjustments::scripts::WINDOW_GUARDED_TOKEN_INCREMENT_SCRIPT;
use crate::token_adjustments::store::{TokenAdjustmentError, TokenAdjustments};

const WINDOW_KEY: &str = "{scope:value}:window";
const COUNTER_KEY: &str = "{scope:value}:tokens";

fn router_adjustments(
    cache: &Arc<FakeCache>,
    usage: &Arc<FakeUsage>,
    script: Option<&Arc<FakeScript>>,
) -> TokenAdjustments {
    adjustments_with(
        Arc::clone(cache),
        Arc::clone(usage),
        None,
        script.map(script_handle),
        None,
    )
}

fn half_guarded(window_key: Option<&str>, expected: Option<&str>) -> ReservationAwareIncrement {
    let mut operation = unguarded_operation(COUNTER_KEY, 2.0);
    operation.window_key = window_key.map(str::to_owned);
    operation.expected_window_start = expected.map(str::to_owned);
    operation.reservation_backend = Some(ReservationBackend::Redis);
    operation
}

#[tokio::test]
async fn operations_without_window_metadata_take_the_direct_increment() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let usage = FakeUsage::new();
    let script = FakeScript::new(WINDOW_GUARDED_TOKEN_INCREMENT_SCRIPT);
    let adjustments = router_adjustments(&cache, &usage, Some(&script));
    let operations = [
        unguarded_operation("plain", 4.0),
        guarded_operation(COUNTER_KEY, 2.0, ReservationBackend::Redis),
    ];
    apply_reservation_aware_tokens(&adjustments, &operations)
        .await
        .expect("the mixed routing");
    assert_eq!(
        operation_tuples(&usage.increments()),
        vec![("plain".to_owned(), 4.0_f64, Some(60))]
    );
    assert_eq!(script.calls().len(), 1);
    assert_eq!(
        script.calls()[0].0,
        vec![WINDOW_KEY.to_owned(), COUNTER_KEY.to_owned()]
    );
}

#[tokio::test]
async fn an_operation_with_only_half_the_window_metadata_takes_the_direct_increment() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let usage = FakeUsage::new();
    let script = FakeScript::new(WINDOW_GUARDED_TOKEN_INCREMENT_SCRIPT);
    let adjustments = router_adjustments(&cache, &usage, Some(&script));
    let operations = [
        half_guarded(Some(WINDOW_KEY), None),
        half_guarded(None, Some(WINDOW_START)),
    ];
    apply_reservation_aware_tokens(&adjustments, &operations)
        .await
        .expect("the half-guarded routing");
    assert_eq!(
        operation_tuples(&usage.increments()),
        vec![
            (COUNTER_KEY.to_owned(), 2.0_f64, Some(60)),
            (COUNTER_KEY.to_owned(), 2.0_f64, Some(60)),
        ]
    );
    assert!(script.calls().is_empty());
}

#[tokio::test]
async fn a_local_backend_updates_the_local_cache() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let usage = FakeUsage::new();
    let script = FakeScript::new(WINDOW_GUARDED_TOKEN_INCREMENT_SCRIPT);
    cache.seed(WINDOW_KEY, CacheCounterValue::Int(100));
    cache.seed(COUNTER_KEY, CacheCounterValue::Float(10.0));
    let adjustments = router_adjustments(&cache, &usage, Some(&script));
    let operations = [guarded_operation(
        COUNTER_KEY,
        2.0,
        ReservationBackend::Local,
    )];
    apply_reservation_aware_tokens(&adjustments, &operations)
        .await
        .expect("the local routing");
    assert_eq!(
        usage.local_sets(),
        vec![(COUNTER_KEY.to_owned(), 12.0_f64, Some(60))]
    );
    assert!(usage.increments().is_empty());
    assert!(script.calls().is_empty());
}

#[tokio::test]
async fn a_redis_backend_runs_the_guarded_script() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let usage = FakeUsage::new();
    let script = FakeScript::new(WINDOW_GUARDED_TOKEN_INCREMENT_SCRIPT);
    let adjustments = router_adjustments(&cache, &usage, Some(&script));
    let operations = [guarded_operation(
        COUNTER_KEY,
        2.0,
        ReservationBackend::Redis,
    )];
    apply_reservation_aware_tokens(&adjustments, &operations)
        .await
        .expect("the redis routing");
    assert_eq!(script.calls().len(), 1);
    assert!(usage.increments().is_empty());
    assert!(usage.local_sets().is_empty());
}

#[tokio::test]
async fn a_guarded_operation_without_a_backend_runs_the_guarded_script() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let usage = FakeUsage::new();
    let script = FakeScript::new(WINDOW_GUARDED_TOKEN_INCREMENT_SCRIPT);
    let adjustments = router_adjustments(&cache, &usage, Some(&script));
    let mut operation = guarded_operation(COUNTER_KEY, 2.0, ReservationBackend::Redis);
    operation.reservation_backend = None;
    apply_reservation_aware_tokens(&adjustments, &[operation])
        .await
        .expect("the default backend routing");
    assert_eq!(
        script.calls(),
        vec![(
            vec![WINDOW_KEY.to_owned(), COUNTER_KEY.to_owned()],
            vec![
                RedisScalar::Text(WINDOW_START.to_owned()),
                RedisScalar::Text("2".to_owned()),
                RedisScalar::Text("60".to_owned()),
            ],
        )]
    );
}

#[tokio::test]
async fn a_local_failure_stops_the_redis_group() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let usage = FakeUsage::new();
    let script = FakeScript::new(WINDOW_GUARDED_TOKEN_INCREMENT_SCRIPT);
    cache.seed(WINDOW_KEY, CacheCounterValue::Int(100));
    cache.seed(COUNTER_KEY, CacheCounterValue::Text("many".to_owned()));
    let adjustments = router_adjustments(&cache, &usage, Some(&script));
    let operations = [
        guarded_operation(COUNTER_KEY, 2.0, ReservationBackend::Local),
        guarded_operation(COUNTER_KEY, 2.0, ReservationBackend::Redis),
    ];
    let result = apply_reservation_aware_tokens(&adjustments, &operations).await;
    assert!(matches!(result, Err(TokenAdjustmentError::CounterText(_))));
    assert!(script.calls().is_empty());
    assert!(usage.increments().is_empty());
}

#[tokio::test]
async fn an_empty_operation_list_does_nothing() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let usage = FakeUsage::new();
    let script = FakeScript::new(WINDOW_GUARDED_TOKEN_INCREMENT_SCRIPT);
    let adjustments = router_adjustments(&cache, &usage, Some(&script));
    apply_reservation_aware_tokens(&adjustments, &[])
        .await
        .expect("the empty routing");
    assert!(script.calls().is_empty());
    assert!(usage.increments().is_empty());
    assert!(usage.local_sets().is_empty());
}
