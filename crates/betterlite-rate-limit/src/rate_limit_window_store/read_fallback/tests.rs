#![cfg(test)]

use std::sync::Arc;

use super::read_counter_snapshot;
use crate::fail_closed::FailClosedPolicy;
use crate::limiter_records::{CacheCounterValue, ScriptReply};
use crate::rate_limit_window_store::batch_counter_read_script::BATCH_COUNTER_READ_SCRIPT;
use crate::rate_limit_window_store::tests::support::{
    FakeCache, FakeScript, LogCapture, key_list, store_with,
};
use crate::rate_limit_window_store::{CounterScript, RedisCacheKind, WindowStore};

fn disabled() -> FailClosedPolicy {
    FailClosedPolicy { enabled: false }
}

fn enabled() -> FailClosedPolicy {
    FailClosedPolicy { enabled: true }
}

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
async fn a_registered_script_reads_the_redis_values() {
    let script = FakeScript::new(
        BATCH_COUNTER_READ_SCRIPT,
        vec![ScriptReply::Array(vec![ScriptReply::Int(2)])],
    );
    let store = store_with_script(&script, Some(RedisCacheKind::Standalone));
    let values = read_counter_snapshot(&store, &disabled(), &key_list(&["alpha"]))
        .await
        .expect("the counter snapshot");
    assert_eq!(values, Some(vec![Some(CacheCounterValue::Int(2))]));
}

#[tokio::test(flavor = "current_thread")]
async fn a_failed_redis_read_logs_a_warning_and_reads_the_local_mirror() {
    let log = LogCapture::start();
    let script: Arc<dyn CounterScript> =
        FakeScript::new(BATCH_COUNTER_READ_SCRIPT, Vec::new()).failing_after(1);
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    cache.seed("alpha", CacheCounterValue::Int(4));
    let mut store = store_with(Arc::clone(&cache));
    store.counter_read_script = Some(Arc::clone(&script));
    let keys = key_list(&["alpha", "beta"]);
    let values = read_counter_snapshot(&store, &disabled(), &keys)
        .await
        .expect("the local mirror");
    assert_eq!(values, Some(vec![Some(CacheCounterValue::Int(4)), None]));
    assert_eq!(cache.batch_reads(), vec![(keys, true)]);
    let text = log.text();
    assert!(text.contains("WARN"));
    assert!(text.contains("batch_counter_read_script failed, using local mirror"));
}

#[tokio::test]
async fn a_failed_redis_read_with_fail_closed_rejects_the_request() {
    let script: Arc<dyn CounterScript> =
        FakeScript::new(BATCH_COUNTER_READ_SCRIPT, Vec::new()).failing_after(1);
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let mut store = store_with(Arc::clone(&cache));
    store.counter_read_script = Some(Arc::clone(&script));
    let error = read_counter_snapshot(&store, &enabled(), &key_list(&["alpha"]))
        .await
        .expect_err("fail closed rejects the request");
    assert_eq!(error.status_code, 503);
    assert!(cache.batch_reads().is_empty());
}

#[tokio::test]
async fn a_failed_redis_read_with_a_failed_local_mirror_is_none() {
    let script: Arc<dyn CounterScript> =
        FakeScript::new(BATCH_COUNTER_READ_SCRIPT, Vec::new()).failing_after(1);
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    cache.fail_batch_reads();
    let mut store = store_with(Arc::clone(&cache));
    store.counter_read_script = Some(Arc::clone(&script));
    let keys = key_list(&["alpha"]);
    let values = read_counter_snapshot(&store, &disabled(), &keys)
        .await
        .expect("the failed local mirror");
    assert_eq!(values, None);
    assert_eq!(cache.batch_reads(), vec![(keys, true)]);
}

#[tokio::test]
async fn a_failed_cache_read_without_a_script_is_none() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    cache.fail_batch_reads();
    let store = store_with(Arc::clone(&cache));
    let keys = key_list(&["alpha"]);
    let values = read_counter_snapshot(&store, &disabled(), &keys)
        .await
        .expect("the failed cache read");
    assert_eq!(values, None);
    assert_eq!(cache.batch_reads(), vec![(keys, false)]);
}

#[tokio::test]
async fn a_missing_script_reads_the_remote_cache() {
    let cache = FakeCache::new(Some(RedisCacheKind::Standalone));
    let store = store_with(Arc::clone(&cache));
    let keys = key_list(&["alpha"]);
    let values = read_counter_snapshot(&store, &disabled(), &keys)
        .await
        .expect("the cache read");
    assert_eq!(values, Some(vec![None]));
    assert_eq!(cache.batch_reads(), vec![(keys, false)]);
}
