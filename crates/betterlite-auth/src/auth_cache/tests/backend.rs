use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::time::timeout;

use betterlite_models::decisions::Json;
use serde::Serialize;

use betterlite_cache::backend::memory::MemoryCacheOps as _;

use super::redis::{FakeRedis, connect_cache};
use super::support::{
    LocalCache, assert_backend_failure, backend_failure, failure_label, local_cache, lock, memory,
    recording_cache, redis_handle, settings,
};
use crate::auth_cache::backend::{AuthCacheTier as _, PreCall};
use crate::auth_cache::keys::CacheKey;
use crate::auth_cache::{AuthCacheError, WriteOptions, WriteTtl, gateway_cache};

const HEX_DIGITS: usize = 64;

fn hex_key(seed: &str) -> String {
    seed.repeat(HEX_DIGITS)
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn plain() -> WriteOptions {
    WriteOptions {
        ttl: None,
        local_only: false,
        refresh_ttl: false,
    }
}

fn with_ttl(ttl: WriteTtl) -> WriteOptions {
    WriteOptions {
        ttl: Some(ttl),
        local_only: false,
        refresh_ttl: false,
    }
}

fn memory_only() -> WriteOptions {
    WriteOptions {
        ttl: None,
        local_only: true,
        refresh_ttl: false,
    }
}

#[derive(Serialize)]
struct BatchEntry {
    value: i64,
    broken: BTreeMap<Vec<u8>, i64>,
}

fn batch_entry(value: i64) -> BatchEntry {
    BatchEntry {
        value,
        broken: BTreeMap::new(),
    }
}

fn broken_entry(value: i64) -> BatchEntry {
    BatchEntry {
        value,
        broken: BTreeMap::from([(vec![1_u8], 1_i64)]),
    }
}

fn fill_text_entries(cache: &LocalCache, prefix: &str, count: usize) {
    for index in 0..count {
        let key = format!("{prefix}{index}");
        cache
            .set(Some(&key), &text("value"), memory_only())
            .expect("the memory tier accepts the entry");
    }
}

fn fill_hex_entries(cache: &LocalCache, count: usize) {
    for index in 0..count {
        let key = format!("{index:064x}");
        cache
            .set(Some(&key), &text("value"), memory_only())
            .expect("the memory tier accepts the entry");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_memory_partitions_shared_redis() {
    let server = FakeRedis::start().await;
    let redis = connect_cache(server.addr).await;
    let cache = gateway_cache(
        memory(),
        memory(),
        Some(Arc::clone(&redis)),
        settings(Some(60.0_f64)),
        45.0_f64,
    );
    let ordinary_redis = redis_handle(&cache, false);
    let key_redis = redis_handle(&cache, true);
    assert!(Arc::ptr_eq(&ordinary_redis, &key_redis));
    assert!(!Arc::ptr_eq(
        &cache.ordinary.in_memory_cache,
        &cache.key_objects.in_memory_cache
    ));
    let key_object = hex_key("a");
    let ordinary = CacheKey::Tag("prod").render();
    cache.set(Some(&key_object), &text("key"), plain()).unwrap();
    cache
        .set(Some(&ordinary), &text("ordinary"), plain())
        .unwrap();
    let stored: Vec<Vec<String>> = server
        .commands("SET")
        .into_iter()
        .map(|command| command.args)
        .collect();
    assert!(stored.iter().any(|args| args.get(1) == Some(&key_object)));
    assert!(stored.iter().any(|args| args.get(1) == Some(&ordinary)));
}

#[tokio::test]
async fn independent_eviction() {
    let cache = local_cache(Some(60.0_f64));
    cache.update_in_memory_max_size(Some(1));
    let key_object = hex_key("b");
    cache.set(Some(&key_object), &text("key"), plain()).unwrap();
    cache
        .set(Some("tag:first"), &text("first"), plain())
        .unwrap();
    cache
        .set(Some("tag:second"), &text("second"), plain())
        .unwrap();
    assert_eq!(cache.get(&key_object, false), Some(text("key")));
    assert_eq!(cache.get("tag:first", false), None);
    assert_eq!(cache.get("tag:second", false), Some(text("second")));
}

#[tokio::test]
async fn constructor_defaults_delegated() {
    let cache = local_cache(Some(60.0_f64));
    fill_text_entries(&cache, "tag:ordinary:", 201);
    let ordinary = cache.in_memory_cache_for("tag:ordinary:0");
    assert_eq!(ordinary.ttl_entries().unwrap().len(), 200);
    assert_eq!(cache.get("tag:ordinary:0", false), None);
    let oversized = Json::Text("x".repeat(1025 * 1024));
    cache
        .set(Some("tag:oversized"), &oversized, memory_only())
        .unwrap();
    assert_eq!(cache.get("tag:oversized", false), None);
}

#[test]
fn settings_update_ordinary_then_key() {
    let fixture = recording_cache();
    fixture
        .cache
        .update_cache_ttl(Some(30.0_f64), Some(30.0_f64));
    fixture.cache.update_in_memory_max_size(Some(300));
    fixture.cache.attach_redis_cache(None, Some(9.0_f64));
    assert_eq!(
        fixture.operations(),
        [
            "ordinary:update_ttl",
            "key:update_ttl",
            "ordinary:update_max_size",
            "key:update_max_size",
            "ordinary:attach_redis",
            "key:attach_redis",
        ]
    );
}

#[tokio::test]
async fn ttl_none_keeps_previous_zero_updates() {
    let cache = local_cache(Some(60.0_f64));
    cache.update_cache_ttl(Some(5.0_f64), Some(7.0_f64));
    assert_eq!(
        *cache.ordinary.default_in_memory_ttl.read().unwrap(),
        Some(5.0_f64)
    );
    assert_eq!(
        *cache.key_objects.default_redis_ttl.read().unwrap(),
        Some(7.0_f64)
    );
    cache.update_cache_ttl(None, None);
    assert_eq!(
        *cache.ordinary.default_in_memory_ttl.read().unwrap(),
        Some(5.0_f64)
    );
    assert_eq!(
        *cache.key_objects.default_redis_ttl.read().unwrap(),
        Some(7.0_f64)
    );
    cache.update_cache_ttl(Some(0.0_f64), Some(0.0_f64));
    assert_eq!(
        *cache.ordinary.default_in_memory_ttl.read().unwrap(),
        Some(0.0_f64)
    );
    assert_eq!(
        *cache.key_objects.default_redis_ttl.read().unwrap(),
        Some(0.0_f64)
    );
}

#[tokio::test]
async fn max_size_none_resets_both() {
    let cache = local_cache(Some(60.0_f64));
    cache.update_in_memory_max_size(Some(1));
    cache.update_in_memory_max_size(None);
    fill_text_entries(&cache, "tag:ordinary:", 250);
    fill_hex_entries(&cache, 250);
    assert_eq!(
        cache
            .in_memory_cache_for("tag:ordinary:0")
            .ttl_entries()
            .unwrap()
            .len(),
        200
    );
    assert_eq!(
        cache
            .in_memory_cache_for(&hex_key("c"))
            .ttl_entries()
            .unwrap()
            .len(),
        200
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn attach_once_without_backfill() {
    let server = FakeRedis::start().await;
    let redis = connect_cache(server.addr).await;
    let cache = local_cache(Some(60.0_f64));
    cache
        .set(Some("tag:memory"), &text("memory"), memory_only())
        .unwrap();
    cache.attach_redis_cache(Some(Arc::clone(&redis)), Some(9.0_f64));
    assert_eq!(
        *cache.ordinary.default_redis_ttl.read().unwrap(),
        Some(9.0_f64)
    );
    assert_eq!(
        *cache.key_objects.default_redis_ttl.read().unwrap(),
        Some(9.0_f64)
    );
    assert!(server.commands("SET").is_empty());
    let other_server = FakeRedis::start().await;
    let other = connect_cache(other_server.addr).await;
    cache.attach_redis_cache(Some(other), Some(20.0_f64));
    assert_eq!(
        *cache.ordinary.default_redis_ttl.read().unwrap(),
        Some(9.0_f64)
    );
    assert!(Arc::ptr_eq(
        cache.ordinary.redis_cache.read().unwrap().as_ref().unwrap(),
        &redis
    ));
    assert!(server.commands("SET").is_empty());
    cache.attach_redis_cache(None, Some(30.0_f64));
    assert_eq!(
        *cache.ordinary.default_redis_ttl.read().unwrap(),
        Some(9.0_f64)
    );
}

#[tokio::test]
async fn local_only_and_options_forwarded() {
    let fixture = recording_cache();
    drop(fixture.cache.get("tag:read", true));
    assert_eq!(
        lock(&fixture.ordinary).reads,
        [("tag:read".to_owned(), true)]
    );
    fixture
        .cache
        .async_set(
            Some("tag:write"),
            &text("value"),
            with_ttl(WriteTtl { seconds: 30.0_f64 }),
        )
        .await
        .unwrap();
    let recorded = lock(&fixture.ordinary).options.first().copied();
    let recorded_options = recorded.expect("the tier saw the options");
    assert_eq!(recorded_options.ttl, Some(30.0_f64));
    assert!(!recorded_options.local_only);
}

#[tokio::test]
async fn none_key_serializes_before_forwarding() {
    let fixture = recording_cache();
    let failed = fixture
        .cache
        .async_set(None, &broken_entry(1), plain())
        .await;
    assert!(
        matches!(failed, Err(AuthCacheError::Payload(_))),
        "{failed:?}"
    );
    assert!(fixture.operations().is_empty());
    fixture
        .cache
        .async_set(None, &text("value"), plain())
        .await
        .unwrap();
    assert_eq!(fixture.operations(), ["ordinary:async_set"]);
    assert_eq!(lock(&fixture.ordinary).sets[0].0, None);
}

#[test]
fn sync_write_never_batches() {
    let fixture = recording_cache();
    fixture
        .cache
        .set(
            Some(&hex_key("d")),
            &text("key"),
            with_ttl(WriteTtl { seconds: 30.0_f64 }),
        )
        .unwrap();
    assert_eq!(fixture.operations(), ["key:set"]);
    assert!(lock(&fixture.key_objects).pre_calls.is_empty());
}

#[tokio::test]
async fn async_batch_only_numeric_ttl_option() {
    let fixture = recording_cache();
    fixture
        .cache
        .async_set(
            Some("tag:numeric"),
            &text("value"),
            with_ttl(WriteTtl { seconds: 30.0_f64 }),
        )
        .await
        .unwrap();
    assert_eq!(lock(&fixture.ordinary).pre_calls, [Some(30.0_f64)]);
    let other_option = WriteOptions {
        ttl: Some(WriteTtl { seconds: 30.0_f64 }),
        local_only: false,
        refresh_ttl: true,
    };
    fixture
        .cache
        .async_set(Some("tag:other"), &text("value"), other_option)
        .await
        .unwrap();
    assert_eq!(lock(&fixture.ordinary).pre_calls, [Some(30.0_f64)]);
    assert_eq!(
        fixture.operations(),
        [
            "ordinary:set_pre_call",
            "ordinary:async_set",
            "ordinary:async_set"
        ]
    );
}

#[tokio::test]
async fn batch_bool_ttl_accepted() {
    let fixture = recording_cache();
    lock(&fixture.ordinary).pre_call = Some(PreCall::Queued);
    fixture
        .cache
        .async_set(
            Some("tag:boolean"),
            &text("value"),
            with_ttl(WriteTtl::from(true)),
        )
        .await
        .unwrap();
    assert_eq!(lock(&fixture.ordinary).pre_calls, [Some(1.0_f64)]);
    assert_eq!(fixture.operations(), ["ordinary:set_pre_call"]);
}

#[tokio::test]
async fn queued_write_not_repeated_or_awaited() {
    let fixture = recording_cache();
    lock(&fixture.ordinary).pre_call = Some(PreCall::Queued);
    fixture
        .cache
        .async_set(Some("tag:queued"), &text("value"), plain())
        .await
        .unwrap();
    assert_eq!(fixture.operations(), ["ordinary:set_pre_call"]);
}

#[tokio::test]
async fn absent_batch_direct_write() {
    let fixture = recording_cache();
    lock(&fixture.ordinary).pre_call = Some(PreCall::Direct);
    fixture
        .cache
        .async_set(Some("tag:direct"), &text("value"), plain())
        .await
        .unwrap();
    assert_eq!(
        fixture.operations(),
        ["ordinary:set_pre_call", "ordinary:async_set"]
    );
}

#[tokio::test]
async fn single_delete_partition_and_result() {
    let fixture = recording_cache();
    let key_object = hex_key("e");
    fixture.cache.tier(&key_object).delete(&key_object).unwrap();
    assert_eq!(fixture.operations(), ["key:delete"]);
    let ordinary_key = CacheKey::Tag("gone").render();
    lock(&fixture.ordinary).delete_failure = Some(backend_failure("ordinary delete"));
    let error = fixture
        .cache
        .tier(&ordinary_key)
        .delete(&ordinary_key)
        .unwrap_err();
    assert_eq!(failure_label(&error), Some("ordinary delete"));
    let async_key = hex_key("f");
    fixture
        .cache
        .tier(&async_key)
        .async_delete(&async_key)
        .await
        .unwrap();
    assert_eq!(
        fixture.operations(),
        ["key:delete", "ordinary:delete", "key:async_delete"]
    );
}

#[test]
fn single_pre_call_delete_partition_and_result() {
    let fixture = recording_cache();
    let key_object = hex_key("d");
    let queued = fixture
        .cache
        .tier(&key_object)
        .delete_pre_call(&key_object)
        .unwrap();
    assert!(queued.is_none());
    assert_eq!(fixture.operations(), ["key:delete_pre_call"]);
    let ordinary_key = CacheKey::Tag("gone").render();
    let ordinary = fixture
        .cache
        .tier(&ordinary_key)
        .delete_pre_call(&ordinary_key)
        .unwrap();
    assert!(ordinary.is_none());
    assert_eq!(
        fixture.operations(),
        ["key:delete_pre_call", "ordinary:delete_pre_call"]
    );
}

#[test]
fn delete_pre_call_without_batch_returns_none() {
    let cache = local_cache(Some(60.0_f64));
    let queued = cache.tier("tag:gone").delete_pre_call("tag:gone").unwrap();
    assert!(queued.is_none());
}

#[tokio::test]
async fn batch_delete_attempts_both_empty_groups() {
    let fixture = recording_cache();
    fixture.cache.async_delete_keys(&[]).await.unwrap();
    assert_eq!(
        fixture.operations(),
        ["key:delete_keys", "ordinary:delete_keys"]
    );
    assert_eq!(
        lock(&fixture.key_objects).delete_batches,
        [Vec::<String>::new()]
    );
    assert_eq!(
        lock(&fixture.ordinary).delete_batches,
        [Vec::<String>::new()]
    );
}

#[tokio::test]
async fn batch_delete_first_partition_error_after_both() {
    let fixture = recording_cache();
    lock(&fixture.ordinary).delete_keys_failure = Some(backend_failure("ordinary batch"));
    lock(&fixture.key_objects).delete_keys_failure = Some(backend_failure("key batch"));
    let keys = [
        CacheKey::Tag("gone").render(),
        hex_key("a"),
        CacheKey::ProjectId("gone").render(),
    ];
    let result = fixture.cache.async_delete_keys(&keys).await;
    assert_backend_failure(result, "key batch");
    assert_eq!(
        fixture.operations(),
        ["key:delete_keys", "ordinary:delete_keys"]
    );
    assert_eq!(lock(&fixture.ordinary).delete_batches[0].len(), 2);
    assert_eq!(lock(&fixture.key_objects).delete_batches[0].len(), 1);
}

#[tokio::test]
async fn batch_delete_cancel_propagates() {
    let fixture = recording_cache();
    lock(&fixture.key_objects).hang_delete_keys = true;
    let keys = [CacheKey::Tag("gone").render(), hex_key("a")];
    let cancelled = timeout(
        Duration::from_millis(20),
        fixture.cache.async_delete_keys(&keys),
    )
    .await;
    let _elapsed = cancelled.expect_err("the whole call must be cancelled");
    assert_eq!(
        fixture.operations(),
        ["key:delete_keys", "ordinary:delete_keys"]
    );
}

#[tokio::test]
async fn batch_write_serializes_all_before_mutation() {
    let fixture = recording_cache();
    let entries = vec![
        ("tag:good".to_owned(), batch_entry(1)),
        ("tag:bad".to_owned(), broken_entry(2)),
    ];
    let result = fixture.cache.async_set_pipeline(&entries, plain()).await;
    assert!(
        matches!(result, Err(AuthCacheError::Payload(_))),
        "{result:?}"
    );
    assert!(fixture.operations().is_empty());
}

#[tokio::test]
async fn batch_write_key_partition_first() {
    let fixture = recording_cache();
    let entries = vec![
        (CacheKey::Tag("one").render(), batch_entry(1)),
        (hex_key("a"), batch_entry(2)),
        (CacheKey::Tag("two").render(), batch_entry(3)),
    ];
    fixture
        .cache
        .async_set_pipeline(&entries, plain())
        .await
        .unwrap();
    assert_eq!(
        fixture.operations(),
        ["key:set_pipeline", "ordinary:set_pipeline"]
    );
    assert_eq!(lock(&fixture.key_objects).pipelines[0].len(), 1);
    let ordinary = lock(&fixture.ordinary);
    assert_eq!(ordinary.pipelines[0].len(), 2);
    assert_eq!(ordinary.pipelines[0][0].0, CacheKey::Tag("one").render());
    assert_eq!(ordinary.pipelines[0][1].0, CacheKey::Tag("two").render());
}

#[tokio::test]
async fn batch_write_preserves_duplicates() {
    let fixture = recording_cache();
    let key = CacheKey::Tag("duplicate").render();
    let entries = vec![
        (key.clone(), batch_entry(1)),
        (key.clone(), batch_entry(2)),
        (key, batch_entry(3)),
    ];
    fixture
        .cache
        .async_set_pipeline(&entries, plain())
        .await
        .unwrap();
    let ordinary = lock(&fixture.ordinary);
    let names: Vec<String> = ordinary.pipelines[0]
        .iter()
        .map(|(name, _value)| name.clone())
        .collect();
    drop(ordinary);
    assert_eq!(names.len(), 3);
    assert!(names.iter().all(|name| name == &entries[0].0));
}

#[tokio::test]
async fn batch_write_skips_empty_partition() {
    let ordinary_only = recording_cache();
    let entries = vec![(CacheKey::Tag("one").render(), batch_entry(1))];
    ordinary_only
        .cache
        .async_set_pipeline(&entries, plain())
        .await
        .unwrap();
    assert_eq!(ordinary_only.operations(), ["ordinary:set_pipeline"]);
    let key_only = recording_cache();
    let key_entries = vec![(hex_key("a"), batch_entry(1))];
    key_only
        .cache
        .async_set_pipeline(&key_entries, plain())
        .await
        .unwrap();
    assert_eq!(key_only.operations(), ["key:set_pipeline"]);
}

#[tokio::test]
async fn key_partition_failure_skips_other() {
    let fixture = recording_cache();
    lock(&fixture.key_objects).pipeline_failure = Some(backend_failure("key pipeline"));
    let entries = vec![
        (CacheKey::Tag("one").render(), batch_entry(1)),
        (hex_key("a"), batch_entry(2)),
    ];
    let result = fixture.cache.async_set_pipeline(&entries, plain()).await;
    assert_backend_failure(result, "key pipeline");
    assert_eq!(fixture.operations(), ["key:set_pipeline"]);
}

#[tokio::test]
async fn later_partition_failure_keeps_key_write() {
    let fixture = recording_cache();
    lock(&fixture.ordinary).pipeline_failure = Some(backend_failure("ordinary pipeline"));
    let entries = vec![
        (CacheKey::Tag("one").render(), batch_entry(1)),
        (hex_key("a"), batch_entry(2)),
    ];
    let result = fixture.cache.async_set_pipeline(&entries, plain()).await;
    assert_backend_failure(result, "ordinary pipeline");
    assert_eq!(
        fixture.operations(),
        ["key:set_pipeline", "ordinary:set_pipeline"]
    );
    assert_eq!(lock(&fixture.key_objects).pipelines[0].len(), 1);
}
