#![cfg(test)]

use std::future::Future as _;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Waker};

use betterlite_redis::cache_primitives::protocol::RedisScalar;

use super::support::{
    FIRST_INEXACT_COUNT, FakeCache, FakeScript, FakeUsage, WINDOW_START, adjustments_with,
    guarded_operation, operation_tuples, script_handle, unguarded_operation,
};
use crate::limiter_records::{CacheCounterValue, ReservationAwareIncrement, ReservationBackend};
use crate::rate_limit_window_store::RedisCacheKind;
use crate::token_adjustments::guarded::{apply_local_guarded_tokens, apply_redis_guarded_tokens};
use crate::token_adjustments::scripts::WINDOW_GUARDED_TOKEN_INCREMENT_SCRIPT;
use crate::token_adjustments::store::{TokenAdjustmentError, TokenAdjustments};

const WINDOW_KEY: &str = "{scope:value}:window";
const COUNTER_KEY: &str = "{scope:value}:tokens";

fn local_adjustments(cache: &Arc<FakeCache>, usage: &Arc<FakeUsage>) -> TokenAdjustments {
    adjustments_with(Arc::clone(cache), Arc::clone(usage), None, None, None)
}

fn redis_adjustments(
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

fn with_expected(key: &str, increment: f64, expected: &str) -> ReservationAwareIncrement {
    let mut operation = guarded_operation(key, increment, ReservationBackend::Local);
    operation.expected_window_start = Some(expected.to_owned());
    operation
}

#[tokio::test]
async fn the_local_increment_adds_the_delta_to_the_cached_counter() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let usage = FakeUsage::new();
    cache.seed(WINDOW_KEY, CacheCounterValue::Text(WINDOW_START.to_owned()));
    cache.seed(COUNTER_KEY, CacheCounterValue::Float(12.5));
    let adjustments = local_adjustments(&cache, &usage);
    let operations = [guarded_operation(
        COUNTER_KEY,
        0.25,
        ReservationBackend::Local,
    )];
    apply_local_guarded_tokens(&adjustments, &operations)
        .await
        .expect("the matching window");
    assert_eq!(
        usage.local_sets(),
        vec![(COUNTER_KEY.to_owned(), 12.75_f64, Some(60))]
    );
    assert_eq!(
        cache.reads(),
        vec![
            (WINDOW_KEY.to_owned(), true),
            (COUNTER_KEY.to_owned(), true)
        ]
    );
}

#[tokio::test]
async fn a_local_adjustment_waits_for_the_check_and_increment_lock() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let usage = FakeUsage::new();
    cache.seed(WINDOW_KEY, CacheCounterValue::Int(100));
    cache.seed(COUNTER_KEY, CacheCounterValue::Float(12.5));
    let adjustments = local_adjustments(&cache, &usage);
    let guard = adjustments.windows.check_and_increment_lock.lock().await;
    let operations = [guarded_operation(
        COUNTER_KEY,
        0.25,
        ReservationBackend::Local,
    )];
    let adjustment = apply_local_guarded_tokens(&adjustments, &operations);
    let mut pinned = pin!(adjustment);
    let mut context = Context::from_waker(Waker::noop());
    assert!(pinned.as_mut().poll(&mut context).is_pending());
    assert!(usage.local_sets().is_empty());
    drop(guard);
    pinned.await.expect("the locked local adjustment");
    assert_eq!(
        usage.local_sets(),
        vec![(COUNTER_KEY.to_owned(), 12.75_f64, Some(60))]
    );
}

#[tokio::test]
async fn a_local_increment_without_a_counter_starts_from_zero() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let usage = FakeUsage::new();
    cache.seed(WINDOW_KEY, CacheCounterValue::Int(100));
    let adjustments = local_adjustments(&cache, &usage);
    let operations = [guarded_operation(
        COUNTER_KEY,
        4.0,
        ReservationBackend::Local,
    )];
    apply_local_guarded_tokens(&adjustments, &operations)
        .await
        .expect("the matching window");
    assert_eq!(
        usage.local_sets(),
        vec![(COUNTER_KEY.to_owned(), 4.0_f64, Some(60))]
    );
}

#[tokio::test]
async fn an_integer_counter_is_added_as_a_float() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let usage = FakeUsage::new();
    cache.seed(WINDOW_KEY, CacheCounterValue::Int(100));
    cache.seed(COUNTER_KEY, CacheCounterValue::Int(12));
    let adjustments = local_adjustments(&cache, &usage);
    let operations = [guarded_operation(
        COUNTER_KEY,
        0.5,
        ReservationBackend::Local,
    )];
    apply_local_guarded_tokens(&adjustments, &operations)
        .await
        .expect("the matching window");
    assert_eq!(
        usage.local_sets(),
        vec![(COUNTER_KEY.to_owned(), 12.5_f64, Some(60))]
    );
}

#[tokio::test]
async fn a_counter_text_with_space_around_it_parses() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let usage = FakeUsage::new();
    cache.seed(WINDOW_KEY, CacheCounterValue::Int(100));
    cache.seed(COUNTER_KEY, CacheCounterValue::Text(" 7.5 ".to_owned()));
    let adjustments = local_adjustments(&cache, &usage);
    let operations = [guarded_operation(
        COUNTER_KEY,
        1.0,
        ReservationBackend::Local,
    )];
    apply_local_guarded_tokens(&adjustments, &operations)
        .await
        .expect("the matching window");
    assert_eq!(
        usage.local_sets(),
        vec![(COUNTER_KEY.to_owned(), 8.5_f64, Some(60))]
    );
}

#[tokio::test]
async fn a_window_that_moved_on_skips_the_local_increment() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let usage = FakeUsage::new();
    cache.seed(WINDOW_KEY, CacheCounterValue::Text("101".to_owned()));
    cache.seed(COUNTER_KEY, CacheCounterValue::Float(12.5));
    let adjustments = local_adjustments(&cache, &usage);
    let operations = [guarded_operation(
        COUNTER_KEY,
        1.0,
        ReservationBackend::Local,
    )];
    apply_local_guarded_tokens(&adjustments, &operations)
        .await
        .expect("a moved window is not an error");
    assert!(usage.local_sets().is_empty());
    assert_eq!(cache.reads(), vec![(WINDOW_KEY.to_owned(), true)]);
}

#[tokio::test]
async fn a_missing_window_skips_the_local_increment() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let usage = FakeUsage::new();
    let adjustments = local_adjustments(&cache, &usage);
    let operations = [guarded_operation(
        COUNTER_KEY,
        1.0,
        ReservationBackend::Local,
    )];
    apply_local_guarded_tokens(&adjustments, &operations)
        .await
        .expect("a missing window is not an error");
    assert!(usage.local_sets().is_empty());
}

#[tokio::test]
async fn a_window_start_that_is_not_a_number_skips_the_local_increment() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let usage = FakeUsage::new();
    cache.seed(WINDOW_KEY, CacheCounterValue::Int(100));
    let adjustments = local_adjustments(&cache, &usage);
    let operations = [with_expected(COUNTER_KEY, 1.0, "soon")];
    apply_local_guarded_tokens(&adjustments, &operations)
        .await
        .expect("an unreadable start is not an error");
    assert!(usage.local_sets().is_empty());
}

#[tokio::test]
async fn an_active_window_that_is_not_a_number_skips_the_local_increment() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let usage = FakeUsage::new();
    cache.seed(WINDOW_KEY, CacheCounterValue::Text("soon".to_owned()));
    let adjustments = local_adjustments(&cache, &usage);
    let operations = [guarded_operation(
        COUNTER_KEY,
        1.0,
        ReservationBackend::Local,
    )];
    apply_local_guarded_tokens(&adjustments, &operations)
        .await
        .expect("an unreadable window is not an error");
    assert!(usage.local_sets().is_empty());
}

#[tokio::test]
async fn operations_without_window_metadata_skip_the_local_increment() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let usage = FakeUsage::new();
    let adjustments = local_adjustments(&cache, &usage);
    let operations = [unguarded_operation(COUNTER_KEY, 1.0)];
    apply_local_guarded_tokens(&adjustments, &operations)
        .await
        .expect("an unguarded operation is skipped");
    assert!(usage.local_sets().is_empty());
    assert!(cache.reads().is_empty());
}

#[tokio::test]
async fn a_counter_text_that_is_not_a_number_is_an_error() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let usage = FakeUsage::new();
    cache.seed(WINDOW_KEY, CacheCounterValue::Int(100));
    cache.seed(COUNTER_KEY, CacheCounterValue::Text("many".to_owned()));
    let adjustments = local_adjustments(&cache, &usage);
    let operations = [guarded_operation(
        COUNTER_KEY,
        1.0,
        ReservationBackend::Local,
    )];
    let result = apply_local_guarded_tokens(&adjustments, &operations).await;
    assert!(matches!(result, Err(TokenAdjustmentError::CounterText(_))));
    assert!(usage.local_sets().is_empty());
}

#[tokio::test]
async fn counter_bytes_that_are_not_utf8_are_an_error() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let usage = FakeUsage::new();
    cache.seed(WINDOW_KEY, CacheCounterValue::Int(100));
    cache.seed(COUNTER_KEY, CacheCounterValue::Bytes(vec![0xff]));
    let adjustments = local_adjustments(&cache, &usage);
    let operations = [guarded_operation(
        COUNTER_KEY,
        1.0,
        ReservationBackend::Local,
    )];
    let result = apply_local_guarded_tokens(&adjustments, &operations).await;
    assert!(matches!(result, Err(TokenAdjustmentError::CounterBytes(_))));
}

#[tokio::test]
async fn a_counter_beyond_the_exact_float_range_is_an_error() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let usage = FakeUsage::new();
    cache.seed(WINDOW_KEY, CacheCounterValue::Int(100));
    cache.seed(COUNTER_KEY, CacheCounterValue::Int(FIRST_INEXACT_COUNT));
    let adjustments = local_adjustments(&cache, &usage);
    let operations = [guarded_operation(
        COUNTER_KEY,
        1.0,
        ReservationBackend::Local,
    )];
    let result = apply_local_guarded_tokens(&adjustments, &operations).await;
    assert!(matches!(
        result,
        Err(TokenAdjustmentError::TokenAmount {
            value: FIRST_INEXACT_COUNT
        })
    ));
}

#[tokio::test]
async fn the_redis_increment_runs_the_script_with_key_pairs() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let usage = FakeUsage::new();
    let script = FakeScript::new(WINDOW_GUARDED_TOKEN_INCREMENT_SCRIPT);
    let adjustments = redis_adjustments(&cache, &usage, Some(&script));
    let operations = [guarded_operation(
        COUNTER_KEY,
        5.0,
        ReservationBackend::Redis,
    )];
    apply_redis_guarded_tokens(&adjustments, &operations).await;
    assert_eq!(
        script.calls(),
        vec![(
            vec![WINDOW_KEY.to_owned(), COUNTER_KEY.to_owned()],
            vec![
                RedisScalar::Text(WINDOW_START.to_owned()),
                RedisScalar::Text("5".to_owned()),
                RedisScalar::Text("60".to_owned()),
            ],
        )]
    );
    assert!(usage.increments().is_empty());
}

#[tokio::test]
async fn a_failed_guarded_script_keeps_a_positive_increment() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let usage = FakeUsage::new();
    let script = FakeScript::new(WINDOW_GUARDED_TOKEN_INCREMENT_SCRIPT);
    script.fail();
    let adjustments = redis_adjustments(&cache, &usage, Some(&script));
    let operations = [guarded_operation(
        COUNTER_KEY,
        5.0,
        ReservationBackend::Redis,
    )];
    apply_redis_guarded_tokens(&adjustments, &operations).await;
    assert_eq!(
        operation_tuples(&usage.increments()),
        vec![(COUNTER_KEY.to_owned(), 5.0_f64, Some(60))]
    );
    assert!(usage.pipelines().is_empty());
}

#[tokio::test]
async fn a_failed_guarded_script_drops_a_negative_increment() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let usage = FakeUsage::new();
    let script = FakeScript::new(WINDOW_GUARDED_TOKEN_INCREMENT_SCRIPT);
    script.fail();
    let adjustments = redis_adjustments(&cache, &usage, Some(&script));
    let operations = [guarded_operation(
        COUNTER_KEY,
        -5.0,
        ReservationBackend::Redis,
    )];
    apply_redis_guarded_tokens(&adjustments, &operations).await;
    assert!(usage.increments().is_empty());
}

#[tokio::test]
async fn a_failed_guarded_script_drops_a_zero_increment() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let usage = FakeUsage::new();
    let script = FakeScript::new(WINDOW_GUARDED_TOKEN_INCREMENT_SCRIPT);
    script.fail();
    let adjustments = redis_adjustments(&cache, &usage, Some(&script));
    let operations = [guarded_operation(
        COUNTER_KEY,
        0.0,
        ReservationBackend::Redis,
    )];
    apply_redis_guarded_tokens(&adjustments, &operations).await;
    assert!(usage.increments().is_empty());
}

#[tokio::test]
async fn without_the_guarded_script_a_positive_increment_still_runs() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let usage = FakeUsage::new();
    let adjustments = redis_adjustments(&cache, &usage, None);
    let operations = [guarded_operation(
        COUNTER_KEY,
        5.0,
        ReservationBackend::Redis,
    )];
    apply_redis_guarded_tokens(&adjustments, &operations).await;
    assert_eq!(
        operation_tuples(&usage.increments()),
        vec![(COUNTER_KEY.to_owned(), 5.0_f64, Some(60))]
    );
}

#[tokio::test]
async fn unguarded_operations_never_reach_the_redis_script() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let usage = FakeUsage::new();
    let script = FakeScript::new(WINDOW_GUARDED_TOKEN_INCREMENT_SCRIPT);
    let adjustments = redis_adjustments(&cache, &usage, Some(&script));
    let operations = [unguarded_operation(COUNTER_KEY, 5.0)];
    apply_redis_guarded_tokens(&adjustments, &operations).await;
    assert!(script.calls().is_empty());
    assert!(usage.increments().is_empty());
}

#[tokio::test]
async fn the_guarded_script_gets_a_zero_ttl_when_the_operation_has_none() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let usage = FakeUsage::new();
    let script = FakeScript::new(WINDOW_GUARDED_TOKEN_INCREMENT_SCRIPT);
    let adjustments = redis_adjustments(&cache, &usage, Some(&script));
    let mut operation = guarded_operation(COUNTER_KEY, 5.0, ReservationBackend::Redis);
    operation.operation.ttl = None;
    apply_redis_guarded_tokens(&adjustments, &[operation]).await;
    assert_eq!(
        script.calls()[0].1,
        vec![
            RedisScalar::Text(WINDOW_START.to_owned()),
            RedisScalar::Text("5".to_owned()),
            RedisScalar::Text("0".to_owned()),
        ]
    );
}
