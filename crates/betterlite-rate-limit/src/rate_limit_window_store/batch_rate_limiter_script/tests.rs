#![cfg(test)]

use std::sync::Arc;

use betterlite_redis::cache_primitives::protocol::RedisScalar;

use super::{BATCH_RATE_LIMITER_SCRIPT, run_batch_rate_limiter_script, window_script_args};
use crate::limiter_records::{CacheCounterValue, ScriptReply};
use crate::rate_limit_window_store::tests::support::{FakeCache, FakeScript, key_list, store_with};
use crate::rate_limit_window_store::{
    CounterScript, RedisCacheKind, WindowStore, WindowStoreError,
};

// LiteLLM's BATCH_RATE_LIMITER_SCRIPT at parallel_request_limiter_v3.py:168-205, comments removed
const PORTED_SCRIPT: &str = "
local results = {}
local now = tonumber(ARGV[1])
local window_size = tonumber(ARGV[2])

for i = 1, #KEYS, 2 do
    local window_key = KEYS[i]
    local counter_key = KEYS[i + 1]
    local increment_value = 1

    local window_start = redis.call('GET', window_key)
    if not window_start or (now - tonumber(window_start)) >= window_size then
        local prefix = string.sub(window_key, 1, -(#':window') - 1)
        redis.call('DEL', prefix .. ':requests', prefix .. ':tokens')
        redis.call('SET', window_key, tostring(now))
        redis.call('SET', counter_key, increment_value)
        redis.call('EXPIRE', window_key, window_size)
        redis.call('EXPIRE', counter_key, window_size)
        table.insert(results, tostring(now))
        table.insert(results, increment_value)
    else
        local counter = redis.call('INCR', counter_key)
        local current_ttl = redis.call('TTL', counter_key)
        if current_ttl == -1 then
            redis.call('EXPIRE', counter_key, window_size)
        end
        table.insert(results, window_start)
        table.insert(results, counter)
    end
end

return results
";

fn register<S>(store: &mut WindowStore, script: &Arc<S>)
where
    S: CounterScript + 'static,
{
    let shared: Arc<dyn CounterScript> = Arc::<S>::clone(script);
    store.rate_limiter_script = Some(shared);
}

fn pair_reply() -> Vec<ScriptReply> {
    vec![ScriptReply::Text("100".to_owned()), ScriptReply::Int(1)]
}

#[test]
fn the_script_is_the_ported_litellm_text_with_its_comments_removed() {
    assert_eq!(BATCH_RATE_LIMITER_SCRIPT, PORTED_SCRIPT);
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

#[tokio::test]
async fn a_failed_script_run_is_returned_and_not_retried() {
    let script = FakeScript::new(BATCH_RATE_LIMITER_SCRIPT, Vec::new()).failing_after(1);
    let mut store = store_with(FakeCache::new(Some(RedisCacheKind::Standalone)));
    register(&mut store, &script);
    let error = run_batch_rate_limiter_script(
        &store,
        &key_list(&["{key:value}:window", "{key:value}:requests"]),
        &window_script_args(100, 60),
    )
    .await
    .expect_err("the script run fails");
    assert!(matches!(error, WindowStoreError::Script(_)));
    assert_eq!(script.calls().len(), 1);
}
