#![cfg(test)]

use std::sync::Arc;

use super::read_counter_values_from_redis;
use crate::limiter_records::{CacheCounterValue, ScriptReply};
use crate::rate_limit_window_store::batch_counter_read_script::BATCH_COUNTER_READ_SCRIPT;
use crate::rate_limit_window_store::tests::support::{FakeCache, FakeScript, key_list, store_with};
use crate::rate_limit_window_store::{CounterScript, RedisCacheKind, WindowStore};

fn store_with_script<S>(script: &Arc<S>, kind: Option<RedisCacheKind>) -> WindowStore
where
    S: CounterScript + 'static,
{
    let mut store = store_with(FakeCache::new(kind));
    let shared: Arc<dyn CounterScript> = Arc::<S>::clone(script);
    store.counter_read_script = Some(shared);
    store
}

#[tokio::test]
async fn values_come_back_in_the_order_of_the_input_keys() {
    let script = FakeScript::new(
        BATCH_COUNTER_READ_SCRIPT,
        vec![
            ScriptReply::Array(vec![ScriptReply::Int(3), ScriptReply::Int(4)]),
            ScriptReply::Array(vec![ScriptReply::Int(5)]),
        ],
    );
    let store = store_with_script(&script, Some(RedisCacheKind::Cluster));
    let keys = key_list(&["{user1000}.following", "foo", "{user1000}.followers"]);
    let values = read_counter_values_from_redis(&store, &keys)
        .await
        .expect("the grouped read");
    assert_eq!(
        values,
        vec![
            Some(CacheCounterValue::Int(3)),
            Some(CacheCounterValue::Int(5)),
            Some(CacheCounterValue::Int(4)),
        ]
    );
    assert_eq!(script.calls().len(), 2);
    assert_eq!(
        script.calls()[0].0,
        key_list(&["{user1000}.following", "{user1000}.followers"])
    );
}

#[tokio::test]
async fn a_standalone_cache_reads_every_key_in_one_call() {
    let script = FakeScript::new(
        BATCH_COUNTER_READ_SCRIPT,
        vec![ScriptReply::Array(vec![
            ScriptReply::Int(1),
            ScriptReply::Nil,
        ])],
    );
    let store = store_with_script(&script, Some(RedisCacheKind::Standalone));
    let values = read_counter_values_from_redis(&store, &key_list(&["alpha", "beta"]))
        .await
        .expect("the cache read");
    assert_eq!(values, vec![Some(CacheCounterValue::Int(1)), None]);
    assert_eq!(script.calls().len(), 1);
    assert_eq!(script.calls()[0].0, key_list(&["alpha", "beta"]));
    assert_eq!(script.calls()[0].1, Vec::new());
}

#[tokio::test]
async fn a_short_group_reply_leaves_the_remaining_keys_missing() {
    let script = FakeScript::new(
        BATCH_COUNTER_READ_SCRIPT,
        vec![ScriptReply::Array(vec![ScriptReply::Int(7)])],
    );
    let store = store_with_script(&script, Some(RedisCacheKind::Standalone));
    let values = read_counter_values_from_redis(&store, &key_list(&["alpha", "beta"]))
        .await
        .expect("the cache read");
    assert_eq!(values, vec![Some(CacheCounterValue::Int(7)), None]);
}

#[tokio::test]
async fn a_missing_read_script_returns_no_values() {
    let store = store_with(FakeCache::new(None));
    assert_eq!(
        read_counter_values_from_redis(&store, &key_list(&["alpha"]))
            .await
            .expect("no registered script"),
        Vec::new()
    );
}
