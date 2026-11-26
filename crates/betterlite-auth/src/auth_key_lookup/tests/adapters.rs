#![cfg(test)]

use std::sync::Arc;

use async_trait::async_trait;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCache, DualCacheSettings};
use betterlite_db::budget_permission_rows::JsonData;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use super::support::{KEY_ROW, auth_from_fields};
use crate::auth_cache::{AuthCache, gateway_cache};
use crate::auth_invalidation::AuthInvalidation;
use crate::auth_invalidation::clock::InvalidationClock;
use crate::auth_invalidation::transport::InvalidationTransport;
use crate::auth_key_lookup::cache::{KeyCache as _, KeyCacheWriter as _};
use crate::auth_key_lookup::permission::{KeyPermissionLookup as _, PermissionSource};
use crate::auth_prefetch::permission::lookup::PermissionError;
use crate::auth_prefetch::permission::store::ObjectPermissionStore;
use crate::auth_pubsub::eviction::LocalEviction;

struct UnusedTransport;

#[async_trait]
impl InvalidationTransport for UnusedTransport {
    async fn publish(&self, _key: &str, _value: Option<f64>, _ttl: Option<f64>) {
        panic!("writing a key never publishes an invalidation");
    }

    async fn evict_and_broadcast(&self, _keys: &[String], _local: &dyn LocalEviction) {
        panic!("writing a key never evicts it");
    }
}

struct FixedClock;

impl InvalidationClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        DateTime::from_timestamp(42, 0).unwrap()
    }
}

fn local_cache() -> Arc<AuthCache<DualCache<LocalMemoryOps>>> {
    let settings = DualCacheSettings {
        default_in_memory_ttl: Some(60.0),
        default_redis_ttl: None,
        batch_cache_expiry: None,
        max_redis_batch_cache_size: 100,
        redis_delete_chunk_size: 100,
    };
    let ordinary = Arc::new(LocalMemoryOps::new(100, Arc::new(|| 42.0)).unwrap());
    let keys = Arc::new(LocalMemoryOps::new(100, Arc::new(|| 42.0)).unwrap());
    Arc::new(gateway_cache(ordinary, keys, None, settings, 60.0))
}

#[tokio::test]
async fn cache_adapters_round_trip() {
    let cache = local_cache();
    let auth_cache = Arc::clone(&cache);
    let writer = AuthInvalidation {
        auth: auth_cache,
        usage: None,
        spend: Arc::new(DualCache::new(
            Arc::new(LocalMemoryOps::new(100, Arc::new(|| 42.0)).unwrap()),
            None,
            DualCacheSettings {
                default_in_memory_ttl: Some(60.0),
                default_redis_ttl: None,
                batch_cache_expiry: None,
                max_redis_batch_cache_size: 100,
                redis_delete_chunk_size: 100,
            },
        )),
        transport: Arc::new(UnusedTransport),
        clock: Arc::new(FixedClock),
        access_group_ttl: 60.0,
    };
    let mut auth = auth_from_fields(KEY_ROW);
    auth.extra.request_route = Some("/v1/chat/completions".to_owned());
    writer.write_key("hashed-key", &mut auth).await.unwrap();
    assert_eq!(auth.view.last_refreshed_at, Some(Decimal::from(42)));
    let hit = cache.read_key("hashed-key").await.unwrap();
    assert_eq!(hit.view.token.key_name.as_deref(), Some("key-1"));
    assert_eq!(hit.view.token.spend, Decimal::new(15, 1));
    assert!(hit.extra.request_route.is_none());
    assert!(cache.read_key("absent").await.is_none());
}

struct PermissionRow;

#[async_trait]
impl ObjectPermissionStore for PermissionRow {
    async fn find_permission(&self, id: &str) -> Result<Option<JsonData>, PermissionError> {
        assert_eq!(id, "op-1");
        Ok(Some(
            serde_json::from_str(r#"{"object_permission_id":"op-1","models":["live"]}"#).unwrap(),
        ))
    }
}

#[tokio::test]
async fn permission_adapter_loads_row() {
    let permissions = PermissionSource {
        cache: local_cache(),
        normal: Arc::new(PermissionRow),
        writer: Arc::new(PermissionRow),
    };
    let found = permissions.permission("op-1", true).await.unwrap().unwrap();
    assert_eq!(found.object_permission_id, "op-1");
}
