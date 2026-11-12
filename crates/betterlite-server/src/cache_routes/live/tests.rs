#![cfg(test)]

use std::sync::Arc;

use betterlite_cache::configuration::cache::{Backend, Cache};
use betterlite_cache::types::cache_type::LiteLlmCacheType;
use betterlite_models::decisions::Json;

use crate::cache_routes::admin::CacheAdmin as _;

use super::super::redis_support::{StubBackend, local_cache, overrides, unreachable_redis_cache};
use super::super::support_tests::parsed_cache_params;
use super::{CacheHandle, LiveCache, LiveResponseCache as _, SlotCache, wire_name};

fn with_injected_redis_type() -> Cache {
    let mut cache = local_cache();
    cache.cache_type = LiteLlmCacheType::Redis;
    cache.backend = Backend::Injected(Arc::new(StubBackend));
    cache
}

#[test]
fn the_slot_returns_the_cache_it_holds() {
    let slot = SlotCache {
        get: || {
            Some(LiveCache {
                cache: Arc::new(local_cache()),
                redis: overrides(),
            })
        },
        clock: Arc::new(|| 1_700_000_000.0_f64),
    };
    let cache = slot.current().expect("the slot holds a cache");
    assert_eq!(cache.cache_type(), "local");
}

#[test]
fn the_slot_returns_none_without_a_cache() {
    let slot = SlotCache {
        get: || None::<LiveCache>,
        clock: Arc::new(|| 1_700_000_000.0_f64),
    };
    assert!(slot.current().is_none());
}

#[test]
fn the_handle_reports_a_local_cache() {
    let handle = CacheHandle {
        cache: Arc::new(local_cache()),
        redis: overrides(),
        clock: Arc::new(|| 1_700_000_000.0_f64),
    };
    assert_eq!(handle.cache_type(), "local");
    let params = parsed_cache_params(&handle.masked_cache_params());
    assert_eq!(params.get("type"), Some(&Json::Text("local".to_owned())));
    assert!(handle.health_check_params().redis_kwargs.is_none());
    assert!(handle.redis_admin().is_none());
}

#[tokio::test]
async fn the_handle_deletes_keys_of_a_local_cache() {
    let handle = CacheHandle {
        cache: Arc::new(local_cache()),
        redis: overrides(),
        clock: Arc::new(|| 1_700_000_000.0_f64),
    };
    handle
        .delete_cache_keys(&["key".to_owned()])
        .await
        .expect("a local cache ignores the delete");
}

#[tokio::test]
async fn the_handle_ping_reports_false_without_a_redis_backend() {
    let handle = CacheHandle {
        cache: Arc::new(with_injected_redis_type()),
        redis: overrides(),
        clock: Arc::new(|| 1_700_000_000.0_f64),
    };
    assert_eq!(handle.ping().await.ok(), Some(false));
}

#[tokio::test]
async fn the_handle_reports_a_test_write_that_cannot_be_prepared() {
    let mut cache = local_cache();
    cache.redis_flush_size = Some(usize::MAX);
    let handle = CacheHandle {
        cache: Arc::new(cache),
        redis: overrides(),
        clock: Arc::new(|| 1_700_000_000.0_f64),
    };
    handle
        .add_test_key()
        .await
        .expect_err("the flush size does not fit the cache envelope");
}

#[tokio::test]
async fn the_handle_reports_a_redis_cache_and_its_failures() {
    let handle = CacheHandle {
        cache: Arc::new(unreachable_redis_cache().await),
        redis: overrides(),
        clock: Arc::new(|| 1_700_000_000.0_f64),
    };
    assert_eq!(handle.cache_type(), "redis");
    assert!(handle.health_check_params().redis_kwargs.is_some());
    handle.ping().await.expect_err("no server answers");
    handle
        .delete_cache_keys(&["key".to_owned()])
        .await
        .expect_err("no server answers");
    handle.add_test_key().await.expect("the write is swallowed");
    let admin = handle.redis_admin().expect("a redis admin");
    admin.client_list().expect_err("no server answers");
    admin.info().expect_err("no server answers");
    admin.flushall().expect_err("no server answers");
}

#[test]
fn the_wire_names_cover_every_cache_type() {
    let names = [
        wire_name(LiteLlmCacheType::Local),
        wire_name(LiteLlmCacheType::Redis),
        wire_name(LiteLlmCacheType::RedisSemantic),
        wire_name(LiteLlmCacheType::ValkeySemantic),
        wire_name(LiteLlmCacheType::S3),
        wire_name(LiteLlmCacheType::Disk),
        wire_name(LiteLlmCacheType::QdrantSemantic),
        wire_name(LiteLlmCacheType::AzureBlob),
        wire_name(LiteLlmCacheType::Gcs),
    ];
    assert_eq!(
        names,
        [
            "local",
            "redis",
            "redis-semantic",
            "valkey-semantic",
            "s3",
            "disk",
            "qdrant-semantic",
            "azure-blob",
            "gcs",
        ]
    );
}
