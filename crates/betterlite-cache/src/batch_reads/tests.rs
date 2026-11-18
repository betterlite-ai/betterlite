#![cfg(test)]

#[path = "tests/support.rs"]
mod support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_models::decisions::Json;
use betterlite_redis::batch::RedisBatch;
use betterlite_redis::cache_primitives::decode::CachedValue;
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::redis_cache_values::error::RedisCacheValuesError;
use serde_json::Number;
use tokio_util::task::TaskTracker;
use tracing::instrument::WithSubscriber;
use tracing::subscriber::with_default;

use self::support::redis::{
    FakeRedis, RejectingGate, Reply, connect_cache, connect_cache_with_gate, local_memory,
    settings_for_tests, write,
};
use self::support::{FailingMemory, capture_logs, captured};
use super::shared::shared_read_failure;
use super::{BatchReadOptions, BatchReads, BatchReadsError, PendingBatchRead};
use crate::backend::memory::MemoryCacheOps;
use crate::backend::memory::local::LocalMemoryOps;
use crate::dual_cache::{DualCache, DualCacheSettings, Ttl};

const THROTTLED: BatchReadOptions = BatchReadOptions {
    local_only: false,
    throttle_redis: true,
};
const UNTHROTTLED: BatchReadOptions = BatchReadOptions {
    local_only: false,
    throttle_redis: false,
};

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn keys(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn access_time<Memory>(cache: &DualCache<Memory>, key: &str) -> Option<f64>
where
    Memory: MemoryCacheOps,
{
    let times = cache
        .last_redis_batch_access_time
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    times.entries.get(key).copied()
}

fn settings_with_expiry(expiry: f64) -> DualCacheSettings {
    DualCacheSettings {
        batch_cache_expiry: Some(expiry),
        ..settings_for_tests(Some(30.0))
    }
}

async fn with_settings(
    server: &FakeRedis,
    settings: DualCacheSettings,
) -> (DualCache<LocalMemoryOps>, Arc<RedisCache>) {
    let redis = connect_cache(server.addr).await;
    let cache = DualCache::new(local_memory(1000.0), Some(Arc::clone(&redis)), settings);
    (cache, redis)
}

async fn with_redis(server: &FakeRedis) -> (DualCache<LocalMemoryOps>, Arc<RedisCache>) {
    with_settings(server, settings_for_tests(Some(30.0))).await
}

async fn with_rejecting_redis(server: &FakeRedis) -> DualCache<LocalMemoryOps> {
    let redis = connect_cache_with_gate(server.addr, Arc::new(RejectingGate)).await;
    DualCache::new(
        local_memory(1000.0),
        Some(redis),
        settings_for_tests(Some(30.0)),
    )
}

fn batch_for(redis: &Arc<RedisCache>) -> Arc<RedisBatch> {
    Arc::new(RedisBatch::new(
        Arc::clone(redis),
        "b4294-tests",
        TaskTracker::new(),
    ))
}

#[test]
fn reserve_skips_memory_hits() {
    let cache = DualCache::new(local_memory(1000.0), None, settings_for_tests(Some(30.0)));
    let result = vec![Some(text("served")), None];
    let (due, previous) = cache.reserve_redis_batch_keys(100.0, &keys(&["hit", "miss"]), &result);
    assert_eq!(due, keys(&["miss"]));
    assert_eq!(previous, BTreeMap::from([("miss".to_owned(), None)]));
    assert_eq!(access_time(&cache, "hit"), None);
    assert_eq!(access_time(&cache, "miss"), Some(100.0));
}

#[test]
fn reserve_first_read_is_due() {
    let cache = DualCache::new(local_memory(1000.0), None, settings_for_tests(Some(30.0)));
    let (due, previous) = cache.reserve_redis_batch_keys(100.0, &keys(&["key"]), &[None]);
    assert_eq!(due, keys(&["key"]));
    assert_eq!(previous, BTreeMap::from([("key".to_owned(), None)]));
    assert_eq!(access_time(&cache, "key"), Some(100.0));
}

#[test]
fn reserve_inside_the_expiry_stays_throttled() {
    let cache = DualCache::new(local_memory(1000.0), None, settings_for_tests(Some(30.0)));
    let reserved = keys(&["key"]);
    let (first, _) = cache.reserve_redis_batch_keys(100.0, &reserved, &[None]);
    assert_eq!(first, reserved);
    let (second, previous) = cache.reserve_redis_batch_keys(105.0, &reserved, &[None]);
    assert!(second.is_empty());
    assert!(previous.is_empty());
    assert_eq!(access_time(&cache, "key"), Some(100.0));
}

#[test]
fn reserve_at_the_expiry_is_due() {
    let cache = DualCache::new(local_memory(1000.0), None, settings_for_tests(Some(30.0)));
    let reserved = keys(&["key"]);
    cache.reserve_redis_batch_keys(100.0, &reserved, &[None]);
    let (due, previous) = cache.reserve_redis_batch_keys(110.0, &reserved, &[None]);
    assert_eq!(due, reserved);
    assert_eq!(previous, BTreeMap::from([("key".to_owned(), Some(100.0))]));
    assert_eq!(access_time(&cache, "key"), Some(110.0));
}

#[test]
fn reserve_past_the_expiry_is_due() {
    let cache = DualCache::new(local_memory(1000.0), None, settings_for_tests(Some(30.0)));
    let reserved = keys(&["key"]);
    cache.reserve_redis_batch_keys(100.0, &reserved, &[None]);
    let (due, _) = cache.reserve_redis_batch_keys(111.0, &reserved, &[None]);
    assert_eq!(due, reserved);
    assert_eq!(access_time(&cache, "key"), Some(111.0));
}

#[test]
fn reserve_rollback_restores_previous_time() {
    let cache = DualCache::new(local_memory(1000.0), None, settings_for_tests(Some(30.0)));
    let reserved = keys(&["key"]);
    cache.reserve_redis_batch_keys(100.0, &reserved, &[None]);
    let (_due, previous) = cache.reserve_redis_batch_keys(200.0, &reserved, &[None]);
    assert_eq!(access_time(&cache, "key"), Some(200.0));
    cache.rollback_redis_batch_keys(&previous);
    assert_eq!(access_time(&cache, "key"), Some(100.0));
}

#[test]
fn reserve_rollback_removes_new_time() {
    let cache = DualCache::new(local_memory(1000.0), None, settings_for_tests(Some(30.0)));
    let (_due, previous) = cache.reserve_redis_batch_keys(100.0, &keys(&["key"]), &[None]);
    assert_eq!(access_time(&cache, "key"), Some(100.0));
    cache.rollback_redis_batch_keys(&previous);
    assert_eq!(access_time(&cache, "key"), None);
}

#[test]
fn reserve_duplicate_keys_are_due_once() {
    let cache = DualCache::new(local_memory(1000.0), None, settings_for_tests(Some(30.0)));
    let (due, previous) =
        cache.reserve_redis_batch_keys(100.0, &keys(&["key", "key"]), &[None, None]);
    assert_eq!(due, keys(&["key"]));
    assert_eq!(previous, BTreeMap::from([("key".to_owned(), None)]));
}

#[test]
fn reserve_reads_without_redis_returns_empty() {
    let cache = DualCache::new(local_memory(1000.0), None, settings_for_tests(Some(30.0)));
    let (due, previous) = cache.reserve_redis_batch_reads(&keys(&["a", "b"]));
    assert!(due.is_empty());
    assert!(previous.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reserve_reads_skips_memory_hits() {
    let server = FakeRedis::start().await;
    let (cache, _redis) = with_redis(&server).await;
    cache.set_cache("hit", &text("served"), write(None, true));
    let (due, previous) = cache.reserve_redis_batch_reads(&keys(&["hit", "miss"]));
    assert_eq!(due, keys(&["miss"]));
    assert_eq!(previous, BTreeMap::from([("miss".to_owned(), None)]));
    assert_eq!(access_time(&cache, "hit"), None);
    assert!(access_time(&cache, "miss").is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reserve_reads_logs_memory_failure_and_reserves() {
    let server = FakeRedis::start().await;
    let cache = DualCache::new(
        Arc::new(FailingMemory {
            values: Mutex::new(BTreeMap::new()),
            fail_reads: true,
            fail_writes: false,
        }),
        Some(connect_cache(server.addr).await),
        settings_for_tests(None),
    );
    let (subscriber, bytes) = capture_logs();
    let (due, previous) = with_default(subscriber, || {
        cache.reserve_redis_batch_reads(&keys(&["a"]))
    });
    assert_eq!(due, keys(&["a"]));
    assert_eq!(previous, BTreeMap::from([("a".to_owned(), None)]));
    let log = captured(&bytes);
    assert!(
        log.contains("LiteLLM Cache: exception in reserve_redis_batch_reads"),
        "{log}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sync_read_returns_memory_when_every_key_hits() {
    let server = FakeRedis::start().await;
    let (cache, _redis) = with_redis(&server).await;
    cache.set_cache("a", &text("value-a"), write(None, true));
    let read = cache
        .batch_get_cache(&keys(&["a"]), false)
        .expect("the memory read");
    assert_eq!(read, vec![Some(text("value-a"))]);
    assert!(server.commands("MGET").is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sync_read_local_only_skips_redis() {
    let server = FakeRedis::start().await;
    let (cache, _redis) = with_redis(&server).await;
    let read = cache
        .batch_get_cache(&keys(&["a"]), true)
        .expect("the local read");
    assert_eq!(read, vec![None]);
    assert!(server.commands("MGET").is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sync_read_without_redis_returns_memory() {
    let cache = DualCache::new(local_memory(1000.0), None, settings_for_tests(Some(30.0)));
    cache.set_cache("a", &text("value-a"), write(None, true));
    let read = cache
        .batch_get_cache(&keys(&["a", "b"]), false)
        .expect("the memory read");
    assert_eq!(read, vec![Some(text("value-a")), None]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sync_read_backfills_with_redis_values() {
    let server = FakeRedis::start().await;
    server.script(
        "MGET",
        vec![Reply::Array(vec![
            Reply::Bulk("\"from-redis\"".to_owned()),
            Reply::Nil,
        ])],
    );
    let (cache, _redis) = with_redis(&server).await;
    let read = cache
        .batch_get_cache(&keys(&["a", "b"]), false)
        .expect("the redis read");
    assert_eq!(read, vec![Some(text("from-redis")), None]);
    assert_eq!(server.commands("MGET")[0].args, vec!["MGET", "a", "b"]);
    assert_eq!(cache.get_cache("a", true), Some(text("from-redis")));
    assert_eq!(cache.get_cache("b", true), None);
    assert_eq!(
        cache.async_get_ttl("a").await.ok().flatten(),
        Some(Ttl::Expiry(1030.0))
    );
    assert_eq!(cache.async_get_ttl("b").await.ok().flatten(), None);
}

#[test]
fn cache_from_json_keeps_whole_and_fractional_numbers() {
    assert_eq!(
        super::value::cache_from_json(&Json::Number(Number::from(42_i64))),
        Some(CacheValue::Integer(42))
    );
    assert_eq!(
        super::value::cache_from_json(&Json::Number(
            Number::from_f64(1.5).expect("a finite number")
        )),
        Some(CacheValue::Float(1.5))
    );
}

#[test]
fn redis_values_restore_the_decoder_number_object() {
    let stored = Some(CachedValue::Object(BTreeMap::from([(
        "$serde_json::private::Number".to_owned(),
        CachedValue::Text("1.5".to_owned()),
    )])));
    let values = BTreeMap::from([("fraction".to_owned(), stored)]);
    let converted = super::value::redis_values(&values);
    assert_eq!(
        converted.get("fraction"),
        Some(&Some(Json::Number(
            Number::from_f64(1.5).expect("a finite number")
        )))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sync_read_circuit_open_returns_memory_and_rolls_back() {
    let server = FakeRedis::start().await;
    let cache = with_rejecting_redis(&server).await;
    cache.set_cache("a", &text("memory-a"), write(None, true));
    let (reserved, _) = cache.reserve_redis_batch_keys(100.0, &keys(&["b"]), &[None]);
    assert_eq!(reserved, keys(&["b"]));
    let (subscriber, bytes) = capture_logs();
    let read = with_default(subscriber, || {
        cache.batch_get_cache(&keys(&["a", "b"]), false)
    });
    assert_eq!(read, Some(vec![Some(text("memory-a")), None]));
    assert_eq!(access_time(&cache, "b"), Some(100.0));
    let log = captured(&bytes);
    assert!(
        log.contains("LiteLLM Cache: batch_get_cache served from memory only"),
        "{log}"
    );
    assert!(server.commands("MGET").is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sync_read_memory_failure_returns_none() {
    let server = FakeRedis::start().await;
    let cache = DualCache::new(
        Arc::new(FailingMemory {
            values: Mutex::new(BTreeMap::new()),
            fail_reads: true,
            fail_writes: false,
        }),
        Some(connect_cache(server.addr).await),
        settings_for_tests(None),
    );
    let (subscriber, bytes) = capture_logs();
    let read = with_default(subscriber, || cache.batch_get_cache(&keys(&["a"]), false));
    assert!(read.is_none());
    let log = captured(&bytes);
    assert!(
        log.contains("LiteLLM Cache: exception in batch_get_cache"),
        "{log}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_read_all_hit() {
    let server = FakeRedis::start().await;
    let (cache, _redis) = with_redis(&server).await;
    cache.set_cache("a", &text("value-a"), write(None, true));
    let read = cache.async_batch_get_cache(&keys(&["a"]), THROTTLED).await;
    assert_eq!(read, Some(vec![Some(text("value-a"))]));
    assert!(server.commands("MGET").is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_read_throttle_on_skips_inside_expiry() {
    let server = FakeRedis::start().await;
    let (cache, _redis) = with_settings(&server, settings_with_expiry(100.0)).await;
    let read = keys(&["a"]);
    let first = cache.async_batch_get_cache(&read, THROTTLED).await;
    let second = cache.async_batch_get_cache(&read, THROTTLED).await;
    assert_eq!(first, Some(vec![None]));
    assert_eq!(second, Some(vec![None]));
    assert_eq!(server.commands("MGET").len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_read_without_throttle_reads_again() {
    let server = FakeRedis::start().await;
    let (cache, _redis) = with_settings(&server, settings_with_expiry(100.0)).await;
    let read = keys(&["a"]);
    cache.async_batch_get_cache(&read, UNTHROTTLED).await;
    cache.async_batch_get_cache(&read, UNTHROTTLED).await;
    assert_eq!(server.commands("MGET").len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_read_duplicate_keys_fetched_once_when_throttled() {
    let server = FakeRedis::start().await;
    let (cache, _redis) = with_settings(&server, settings_with_expiry(100.0)).await;
    let read = cache
        .async_batch_get_cache(&keys(&["a", "a"]), THROTTLED)
        .await;
    assert_eq!(read, Some(vec![None, None]));
    let commands = server.commands("MGET");
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].args, vec!["MGET", "a"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_read_duplicate_keys_fetched_twice_without_throttle() {
    let server = FakeRedis::start().await;
    let (cache, _redis) = with_settings(&server, settings_with_expiry(100.0)).await;
    let read = cache
        .async_batch_get_cache(&keys(&["a", "a"]), UNTHROTTLED)
        .await;
    assert_eq!(read, Some(vec![None, None]));
    assert_eq!(server.commands("MGET")[0].args, vec!["MGET", "a", "a"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_read_merges_redis_over_memory_misses_only() {
    let server = FakeRedis::start().await;
    server.script(
        "MGET",
        vec![Reply::Array(vec![Reply::Bulk("\"redis-b\"".to_owned())])],
    );
    let (cache, _redis) = with_redis(&server).await;
    cache.set_cache("a", &text("memory-a"), write(None, true));
    let read = cache
        .async_batch_get_cache(&keys(&["a", "b"]), THROTTLED)
        .await;
    assert_eq!(
        read,
        Some(vec![Some(text("memory-a")), Some(text("redis-b"))])
    );
    assert_eq!(server.commands("MGET")[0].args, vec!["MGET", "b"]);
    assert_eq!(cache.get_cache("b", true), Some(text("redis-b")));
    assert_eq!(
        cache.async_get_ttl("b").await.ok().flatten(),
        Some(Ttl::Expiry(1030.0))
    );
}

#[test]
fn async_read_apply_prefers_the_redis_result() {
    let cache = DualCache::new(local_memory(1000.0), None, settings_for_tests(None));
    let pending = PendingBatchRead {
        keys: keys(&["a", "b", "c"]),
        result: vec![Some(text("memory-a")), None, Some(text("memory-c"))],
        redis_keys: keys(&["a", "b", "c"]),
        previous_access_times: BTreeMap::new(),
    };
    let values = BTreeMap::from([
        ("a".to_owned(), Some(text("redis-a"))),
        ("c".to_owned(), None),
    ]);
    let merged = cache
        .apply_batch_get(&pending, Some(&values))
        .expect("the merged read");
    assert_eq!(merged, vec![Some(text("redis-a")), None, None]);
    assert_eq!(cache.get_cache("a", true), Some(text("redis-a")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_read_all_none_redis_result_leaves_result() {
    let server = FakeRedis::start().await;
    server.script("MGET", vec![Reply::Array(vec![Reply::Nil])]);
    let (cache, _redis) = with_redis(&server).await;
    let read = cache.async_batch_get_cache(&keys(&["a"]), THROTTLED).await;
    assert_eq!(read, Some(vec![None]));
    assert_eq!(cache.get_cache("a", true), None);
    assert_eq!(cache.async_get_ttl("a").await.ok().flatten(), None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_read_redis_error_returns_memory() {
    let server = FakeRedis::start().await;
    server.script("MGET", vec![Reply::Error("ERR boom")]);
    let (cache, _redis) = with_redis(&server).await;
    cache.set_cache("a", &text("memory-a"), write(None, true));
    let read = cache
        .async_batch_get_cache(&keys(&["a", "b"]), THROTTLED)
        .await;
    assert_eq!(read, Some(vec![Some(text("memory-a")), None]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_read_circuit_open_returns_memory() {
    let server = FakeRedis::start().await;
    let cache = with_rejecting_redis(&server).await;
    cache.set_cache("a", &text("memory-a"), write(None, true));
    let (reserved, _) = cache.reserve_redis_batch_keys(100.0, &keys(&["b"]), &[None]);
    assert_eq!(reserved, keys(&["b"]));
    let (subscriber, bytes) = capture_logs();
    let read = cache
        .async_batch_get_cache(&keys(&["a", "b"]), THROTTLED)
        .with_subscriber(subscriber)
        .await;
    assert_eq!(read, Some(vec![Some(text("memory-a")), None]));
    assert_eq!(access_time(&cache, "b"), Some(100.0));
    let log = captured(&bytes);
    assert!(
        log.contains("LiteLLM Cache: async_batch_get_cache served from memory only"),
        "{log}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn async_read_memory_failure_returns_none() {
    let server = FakeRedis::start().await;
    let cache = DualCache::new(
        Arc::new(FailingMemory {
            values: Mutex::new(BTreeMap::new()),
            fail_reads: true,
            fail_writes: false,
        }),
        Some(connect_cache(server.addr).await),
        settings_for_tests(None),
    );
    let (subscriber, bytes) = capture_logs();
    let read = cache
        .async_batch_get_cache(&keys(&["a"]), THROTTLED)
        .with_subscriber(subscriber)
        .await;
    assert!(read.is_none());
    let log = captured(&bytes);
    assert!(
        log.contains("LiteLLM Cache: exception in async_batch_get_cache"),
        "{log}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn declared_empty_list_issues_no_mget() {
    let server = FakeRedis::start().await;
    let (cache, redis) = with_redis(&server).await;
    let batch = batch_for(&redis);
    let no_keys: Vec<String> = Vec::new();
    let declared = cache
        .declare_batch_get(&no_keys, &batch)
        .expect("the declared read");
    assert!(declared.result.is_none());
    assert!(declared.pending.redis_keys.is_empty());
    assert!(server.commands("MGET").is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn declared_takes_every_missing_key_without_throttling() {
    let server = FakeRedis::start().await;
    let (cache, redis) = with_redis(&server).await;
    let batch = batch_for(&redis);
    let declared = cache
        .declare_batch_get(&keys(&["a", "a", "b"]), &batch)
        .expect("the declared read");
    assert_eq!(declared.pending.redis_keys, keys(&["a", "a", "b"]));
    assert!(declared.result.is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn declared_resolve_applies_and_backfills() {
    let server = FakeRedis::start().await;
    server.script(
        "MGET",
        vec![Reply::Array(vec![
            Reply::Bulk("\"redis-a\"".to_owned()),
            Reply::Nil,
        ])],
    );
    let (cache, redis) = with_redis(&server).await;
    let batch = batch_for(&redis);
    let declared = cache
        .declare_batch_get(&keys(&["a", "b"]), &batch)
        .expect("the declared read");
    let read = cache
        .async_resolve_batch_get(declared)
        .await
        .expect("the resolved read");
    assert_eq!(read, vec![Some(text("redis-a")), None]);
    assert_eq!(server.commands("MGET")[0].args, vec!["MGET", "a", "b"]);
    assert_eq!(cache.get_cache("a", true), Some(text("redis-a")));
    assert_eq!(
        cache.async_get_ttl("a").await.ok().flatten(),
        Some(Ttl::Expiry(1030.0))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn declared_resolve_propagates_a_batch_error() {
    let server = FakeRedis::start().await;
    let redis = connect_cache_with_gate(server.addr, Arc::new(RejectingGate)).await;
    let cache = DualCache::new(
        local_memory(1000.0),
        Some(Arc::clone(&redis)),
        settings_for_tests(None),
    );
    let batch = batch_for(&redis);
    let declared = cache
        .declare_batch_get(&keys(&["a"]), &batch)
        .expect("the declared read");
    let outcome = cache.async_resolve_batch_get(declared).await;
    assert!(matches!(outcome, Err(BatchReadsError::Batch(_))));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn declared_resolve_without_result_returns_memory() {
    let server = FakeRedis::start().await;
    let (cache, redis) = with_redis(&server).await;
    cache.set_cache("a", &text("memory-a"), write(None, true));
    let batch = batch_for(&redis);
    let declared = cache
        .declare_batch_get(&keys(&["a"]), &batch)
        .expect("the declared read");
    assert!(declared.result.is_none());
    let read = cache
        .async_resolve_batch_get(declared)
        .await
        .expect("the resolved read");
    assert_eq!(read, vec![Some(text("memory-a"))]);
    assert!(server.commands("MGET").is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shared_read_merges_keys_on_one_client() {
    let server = FakeRedis::start().await;
    let (cache_a, redis) = with_redis(&server).await;
    let cache_b = DualCache::new(
        local_memory(1000.0),
        Some(Arc::clone(&redis)),
        settings_for_tests(Some(30.0)),
    );
    server.script(
        "MGET",
        vec![Reply::Array(vec![
            Reply::Bulk("\"redis-a2\"".to_owned()),
            Reply::Bulk("\"redis-b2\"".to_owned()),
        ])],
    );
    cache_a.set_cache("a1", &text("memory-a1"), write(None, true));
    let keys_a = keys(&["a1", "a2"]);
    let keys_b = keys(&["a2", "b2"]);
    let reads = [(&cache_a, keys_a.as_slice()), (&cache_b, keys_b.as_slice())];
    let results = DualCache::async_batch_get_cache_shared(&reads).await;
    assert_eq!(
        results[0],
        Some(vec![Some(text("memory-a1")), Some(text("redis-a2"))])
    );
    assert_eq!(
        results[1],
        Some(vec![Some(text("redis-a2")), Some(text("redis-b2"))])
    );
    let mgets = server.commands("MGET");
    assert_eq!(mgets.len(), 1);
    assert_eq!(mgets[0].args, vec!["MGET", "a2", "b2"]);
    assert_eq!(cache_a.get_cache("a2", true), Some(text("redis-a2")));
    assert_eq!(cache_a.get_cache("b2", true), None);
    assert_eq!(cache_b.get_cache("b2", true), Some(text("redis-b2")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shared_read_with_no_due_keys_issues_no_redis_read() {
    let server = FakeRedis::start().await;
    let (cache_a, redis) = with_redis(&server).await;
    let cache_b = DualCache::new(
        local_memory(1000.0),
        Some(Arc::clone(&redis)),
        settings_for_tests(Some(30.0)),
    );
    cache_a.set_cache("a", &text("memory-a"), write(None, true));
    cache_b.set_cache("b", &text("memory-b"), write(None, true));
    let keys_a = keys(&["a"]);
    let keys_b = keys(&["b"]);
    let reads = [(&cache_a, keys_a.as_slice()), (&cache_b, keys_b.as_slice())];
    let results = DualCache::async_batch_get_cache_shared(&reads).await;
    assert_eq!(results[0], Some(vec![Some(text("memory-a"))]));
    assert_eq!(results[1], Some(vec![Some(text("memory-b"))]));
    assert!(server.commands("MGET").is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shared_read_reads_other_clients_on_their_own() {
    let first = FakeRedis::start().await;
    let second = FakeRedis::start().await;
    let (cache_a, _redis_a) = with_redis(&first).await;
    let (cache_b, _redis_b) = with_redis(&second).await;
    first.script(
        "MGET",
        vec![Reply::Array(vec![Reply::Bulk("\"redis-a\"".to_owned())])],
    );
    second.script(
        "MGET",
        vec![Reply::Array(vec![Reply::Bulk("\"redis-b\"".to_owned())])],
    );
    let keys_a = keys(&["a"]);
    let keys_b = keys(&["b"]);
    let reads = [(&cache_a, keys_a.as_slice()), (&cache_b, keys_b.as_slice())];
    let results = DualCache::async_batch_get_cache_shared(&reads).await;
    assert_eq!(results[0], Some(vec![Some(text("redis-a"))]));
    assert_eq!(results[1], Some(vec![Some(text("redis-b"))]));
    assert_eq!(first.commands("MGET")[0].args, vec!["MGET", "a"]);
    assert_eq!(second.commands("MGET")[0].args, vec!["MGET", "b"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shared_read_without_shared_redis_reads_every_cache_on_its_own() {
    let server = FakeRedis::start().await;
    let memory_only = DualCache::new(local_memory(1000.0), None, settings_for_tests(Some(30.0)));
    let (cache_b, _redis) = with_redis(&server).await;
    server.script(
        "MGET",
        vec![Reply::Array(vec![Reply::Bulk("\"redis-b\"".to_owned())])],
    );
    let keys_a = keys(&["a"]);
    let keys_b = keys(&["b"]);
    let reads = [
        (&memory_only, keys_a.as_slice()),
        (&cache_b, keys_b.as_slice()),
    ];
    let results = DualCache::async_batch_get_cache_shared(&reads).await;
    assert_eq!(results[0], Some(vec![None]));
    assert_eq!(results[1], Some(vec![Some(text("redis-b"))]));
    assert_eq!(server.commands("MGET")[0].args, vec!["MGET", "b"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shared_read_circuit_open_keeps_memory_results() {
    let server = FakeRedis::start().await;
    let redis = connect_cache_with_gate(server.addr, Arc::new(RejectingGate)).await;
    let cache_a = DualCache::new(
        local_memory(1000.0),
        Some(Arc::clone(&redis)),
        settings_for_tests(Some(30.0)),
    );
    let cache_b = DualCache::new(
        local_memory(1000.0),
        Some(Arc::clone(&redis)),
        settings_for_tests(Some(30.0)),
    );
    cache_a.set_cache("a1", &text("memory-a1"), write(None, true));
    let keys_a = keys(&["a1", "a2"]);
    let keys_b = keys(&["b1"]);
    let reads = [(&cache_a, keys_a.as_slice()), (&cache_b, keys_b.as_slice())];
    let (subscriber, bytes) = capture_logs();
    let results = DualCache::async_batch_get_cache_shared(&reads)
        .with_subscriber(subscriber)
        .await;
    assert_eq!(results[0], Some(vec![Some(text("memory-a1")), None]));
    assert_eq!(results[1], Some(vec![None]));
    let log = captured(&bytes);
    assert!(
        log.contains("LiteLLM Cache: async_batch_get_cache_shared served from memory only"),
        "{log}"
    );
    assert!(server.commands("MGET").is_empty());
}

#[test]
fn shared_read_failure_sets_none_only_for_caches_with_redis_keys() {
    let cache = DualCache::new(local_memory(1000.0), None, settings_for_tests(None));
    cache.reserve_redis_batch_keys(100.0, &keys(&["a"]), &[None]);
    let prepared = vec![
        (
            0_usize,
            &cache,
            PendingBatchRead {
                keys: keys(&["a"]),
                result: vec![None],
                redis_keys: keys(&["a"]),
                previous_access_times: BTreeMap::from([("a".to_owned(), None)]),
            },
        ),
        (
            1_usize,
            &cache,
            PendingBatchRead {
                keys: keys(&["b"]),
                result: vec![Some(text("memory-b"))],
                redis_keys: Vec::new(),
                previous_access_times: BTreeMap::new(),
            },
        ),
    ];
    let mut slots: BTreeMap<usize, Option<Vec<Option<Json>>>> = BTreeMap::from([
        (0, Some(vec![None])),
        (1, Some(vec![Some(text("memory-b"))])),
    ]);
    let error = BatchReadsError::RedisValues(RedisCacheValuesError::TtlOutOfRange { seconds: 1.0 });
    let (subscriber, bytes) = capture_logs();
    with_default(subscriber, || {
        shared_read_failure(&error, &prepared, &mut slots)
    });
    assert!(matches!(slots.get(&0), Some(None)));
    assert_eq!(
        slots.get(&1).cloned().flatten(),
        Some(vec![Some(text("memory-b"))])
    );
    assert_eq!(access_time(&cache, "a"), None);
    let log = captured(&bytes);
    assert!(
        log.contains("LiteLLM Cache: exception in async_batch_get_cache_shared"),
        "{log}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shared_read_prepare_failure_leaves_slot_none() {
    let server = FakeRedis::start().await;
    let redis = connect_cache(server.addr).await;
    let working = Arc::new(FailingMemory::default());
    let failing = Arc::new(FailingMemory {
        values: Mutex::new(BTreeMap::new()),
        fail_reads: true,
        fail_writes: false,
    });
    let cache_a = DualCache::new(
        working,
        Some(Arc::clone(&redis)),
        settings_for_tests(Some(30.0)),
    );
    let cache_b = DualCache::new(
        failing,
        Some(Arc::clone(&redis)),
        settings_for_tests(Some(30.0)),
    );
    server.script(
        "MGET",
        vec![Reply::Array(vec![Reply::Bulk("\"redis-a\"".to_owned())])],
    );
    let keys_a = keys(&["a"]);
    let keys_b = keys(&["b"]);
    let reads = [(&cache_a, keys_a.as_slice()), (&cache_b, keys_b.as_slice())];
    let (subscriber, bytes) = capture_logs();
    let results = DualCache::async_batch_get_cache_shared(&reads)
        .with_subscriber(subscriber)
        .await;
    assert_eq!(results[0], Some(vec![Some(text("redis-a"))]));
    assert!(results[1].is_none());
    assert_eq!(server.commands("MGET")[0].args, vec!["MGET", "a"]);
    let log = captured(&bytes);
    assert!(
        log.contains("LiteLLM Cache: exception in async_batch_get_cache_shared"),
        "{log}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shared_read_apply_failure_sets_none_and_logs() {
    let server = FakeRedis::start().await;
    let redis = connect_cache(server.addr).await;
    let working = Arc::new(FailingMemory::default());
    let failing = Arc::new(FailingMemory {
        values: Mutex::new(BTreeMap::new()),
        fail_reads: false,
        fail_writes: true,
    });
    let cache_a = DualCache::new(
        working,
        Some(Arc::clone(&redis)),
        settings_for_tests(Some(30.0)),
    );
    let cache_b = DualCache::new(
        failing,
        Some(Arc::clone(&redis)),
        settings_for_tests(Some(30.0)),
    );
    server.script(
        "MGET",
        vec![Reply::Array(vec![
            Reply::Bulk("\"redis-a\"".to_owned()),
            Reply::Bulk("\"redis-b\"".to_owned()),
        ])],
    );
    let keys_a = keys(&["a"]);
    let keys_b = keys(&["b"]);
    let reads = [(&cache_a, keys_a.as_slice()), (&cache_b, keys_b.as_slice())];
    let (subscriber, bytes) = capture_logs();
    let results = DualCache::async_batch_get_cache_shared(&reads)
        .with_subscriber(subscriber)
        .await;
    assert_eq!(results[0], Some(vec![Some(text("redis-a"))]));
    assert!(results[1].is_none());
    let log = captured(&bytes);
    assert!(
        log.contains("LiteLLM Cache: exception in async_batch_get_cache_shared"),
        "{log}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shared_read_empty_slice_returns_empty() {
    let reads: Vec<(&DualCache<LocalMemoryOps>, &[String])> = Vec::new();
    let results = DualCache::async_batch_get_cache_shared(&reads).await;
    assert!(results.is_empty());
}
