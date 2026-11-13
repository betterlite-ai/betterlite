#![cfg(test)]

use std::sync::Arc;

use betterlite_redis::cache_primitives::protocol::RedisScalar;

use super::{BATCH_RATE_LIMITER_SCRIPT, run_batch_rate_limiter_script, window_script_args};
use crate::limiter_records::{CacheCounterValue, ScriptReply};
use crate::rate_limit_window_store::tests::support::{FakeCache, FakeScript, key_list, store_with};
use crate::rate_limit_window_store::{RedisCacheKind, WindowStore, WindowStoreError};

fn register(store: &mut WindowStore, script: &Arc<FakeScript>) {
    store.rate_limiter_script = Some(Arc::clone(script));
}

fn pair_reply() -> Vec<ScriptReply> {
    vec![ScriptReply::Text("100".to_owned()), ScriptReply::Int(1)]
}

#[test]
fn the_script_pairs_each_window_with_its_counter() {
    assert!(BATCH_RATE_LIMITER_SCRIPT.contains("for i = 1, #KEYS, 2 do"));
    assert!(BATCH_RATE_LIMITER_SCRIPT.contains("local window_key = KEYS[i]"));
    assert!(BATCH_RATE_LIMITER_SCRIPT.contains("local counter_key = KEYS[i + 1]"));
}

#[test]
fn the_script_reads_the_now_and_window_size_arguments() {
    assert!(BATCH_RATE_LIMITER_SCRIPT.contains("local now = tonumber(ARGV[1])"));
    assert!(BATCH_RATE_LIMITER_SCRIPT.contains("local window_size = tonumber(ARGV[2])"));
}

#[test]
fn the_script_resets_an_expired_window() {
    assert!(
        BATCH_RATE_LIMITER_SCRIPT
            .contains("if not window_start or (now - tonumber(window_start)) >= window_size then")
    );
    assert!(BATCH_RATE_LIMITER_SCRIPT.contains("redis.call('SET', window_key, tostring(now))"));
    assert!(BATCH_RATE_LIMITER_SCRIPT.contains("redis.call('SET', counter_key, increment_value)"));
}

#[test]
fn the_script_clears_the_sibling_request_and_token_counters() {
    assert!(
        BATCH_RATE_LIMITER_SCRIPT
            .contains("local prefix = string.sub(window_key, 1, -(#':window') - 1)")
    );
    assert!(
        BATCH_RATE_LIMITER_SCRIPT
            .contains("redis.call('DEL', prefix .. ':requests', prefix .. ':tokens')")
    );
}

#[test]
fn the_script_sets_the_window_and_counter_ttl_on_a_reset() {
    assert!(BATCH_RATE_LIMITER_SCRIPT.contains("redis.call('EXPIRE', window_key, window_size)"));
    assert!(BATCH_RATE_LIMITER_SCRIPT.contains("redis.call('EXPIRE', counter_key, window_size)"));
}

#[test]
fn the_script_increments_within_a_live_window() {
    assert!(BATCH_RATE_LIMITER_SCRIPT.contains("local counter = redis.call('INCR', counter_key)"));
    assert!(BATCH_RATE_LIMITER_SCRIPT.contains("table.insert(results, window_start)"));
    assert!(BATCH_RATE_LIMITER_SCRIPT.contains("table.insert(results, counter)"));
}

#[test]
fn the_script_repairs_a_counter_without_a_ttl() {
    assert!(
        BATCH_RATE_LIMITER_SCRIPT.contains("local current_ttl = redis.call('TTL', counter_key)")
    );
    assert!(BATCH_RATE_LIMITER_SCRIPT.contains("if current_ttl == -1 then"));
}

#[test]
fn the_script_returns_the_window_start_and_counter_of_every_pair() {
    assert!(BATCH_RATE_LIMITER_SCRIPT.contains("table.insert(results, tostring(now))"));
    assert!(BATCH_RATE_LIMITER_SCRIPT.contains("table.insert(results, increment_value)"));
    assert!(BATCH_RATE_LIMITER_SCRIPT.contains("return results"));
}

#[test]
fn the_arguments_are_the_now_and_the_window_size() {
    assert_eq!(
        window_script_args(100, 60),
        vec![
            RedisScalar::Text("100".to_owned()),
            RedisScalar::Text("60".to_owned()),
        ]
    );
}

#[tokio::test]
async fn the_runner_decodes_the_reply_in_pair_order() {
    let script = FakeScript::new(
        BATCH_RATE_LIMITER_SCRIPT,
        vec![ScriptReply::Array(pair_reply())],
    );
    let mut store = store_with(FakeCache::new(Some(RedisCacheKind::Standalone)));
    register(&mut store, &script);
    let values = run_batch_rate_limiter_script(
        &store,
        &key_list(&["{key:value}:window", "{key:value}:requests"]),
        &window_script_args(100, 60),
    )
    .await
    .expect("the batch script run");
    assert_eq!(
        values,
        vec![
            CacheCounterValue::Text("100".to_owned()),
            CacheCounterValue::Int(1),
        ]
    );
}

#[tokio::test]
async fn the_runner_sends_the_keys_and_the_arguments_unchanged() {
    let script = FakeScript::new(
        BATCH_RATE_LIMITER_SCRIPT,
        vec![ScriptReply::Array(pair_reply())],
    );
    let mut store = store_with(FakeCache::new(Some(RedisCacheKind::Standalone)));
    register(&mut store, &script);
    let keys = key_list(&["{key:value}:window", "{key:value}:requests"]);
    run_batch_rate_limiter_script(&store, &keys, &window_script_args(100, 60))
        .await
        .expect("the batch script run");
    assert_eq!(script.calls(), vec![(keys, window_script_args(100, 60))]);
}

#[tokio::test]
async fn an_unregistered_script_is_an_error() {
    let store = store_with(FakeCache::new(None));
    let error = run_batch_rate_limiter_script(&store, &[], &[])
        .await
        .expect_err("no registered script");
    assert!(matches!(
        error,
        WindowStoreError::MissingScript {
            name: "batch_rate_limiter_script"
        }
    ));
}

#[tokio::test]
async fn a_reply_with_a_nil_element_is_an_error() {
    let script = FakeScript::new(
        BATCH_RATE_LIMITER_SCRIPT,
        vec![ScriptReply::Array(vec![ScriptReply::Nil])],
    );
    let mut store = store_with(FakeCache::new(Some(RedisCacheKind::Standalone)));
    register(&mut store, &script);
    let error = run_batch_rate_limiter_script(
        &store,
        &key_list(&["{key:value}:window"]),
        &window_script_args(100, 60),
    )
    .await
    .expect_err("a nil reply element");
    assert!(matches!(error, WindowStoreError::Reply(_)));
}
