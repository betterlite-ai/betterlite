use std::sync::Arc;

use async_trait::async_trait;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCache, DualCacheSettings, MAX_REDIS_BATCH_CACHE_SIZE};
use betterlite_db::resource_rows::access_group::AccessGroupTable;

use super::support::cached_team;
use crate::auth_cache::{AuthCache, gateway_cache};
use crate::auth_invalidation::AuthInvalidation;
use crate::auth_invalidation::clock::SystemClock;
use crate::auth_invalidation::ports::{AuthInvalidationCache, SpendCounterCache};
use crate::auth_invalidation::transport::InvalidationTransport;
use crate::auth_pubsub::eviction::LocalEviction;
use crate::auth_team_lookup::ports::{GatewayTeamCache, TeamCache as _};

struct Silent;

#[async_trait]
impl InvalidationTransport for Silent {
    async fn publish(&self, _cache_key: &str, _new_value: Option<f64>, _ttl: Option<f64>) {}

    async fn evict_and_broadcast(&self, _cache_keys: &[String], _local: &dyn LocalEviction) {}
}

fn memory() -> Arc<LocalMemoryOps> {
    let clock: Arc<dyn Fn() -> f64 + Send + Sync> = Arc::new(|| 100.0_f64);
    Arc::new(LocalMemoryOps::new(1024, clock).unwrap())
}

const fn settings() -> DualCacheSettings {
    DualCacheSettings {
        default_in_memory_ttl: None,
        default_redis_ttl: None,
        batch_cache_expiry: None,
        max_redis_batch_cache_size: MAX_REDIS_BATCH_CACHE_SIZE,
        redis_delete_chunk_size: MAX_REDIS_BATCH_CACHE_SIZE,
    }
}

fn gateway() -> GatewayTeamCache<DualCache<LocalMemoryOps>> {
    let cache = Arc::new(gateway_cache(
        memory(),
        memory(),
        None,
        settings(),
        60.0_f64,
    ));
    let auth: Arc<dyn AuthInvalidationCache> =
        Arc::<AuthCache<DualCache<LocalMemoryOps>>>::clone(&cache);
    let spend: Arc<dyn SpendCounterCache> = Arc::new(DualCache::new(memory(), None, settings()));
    let invalidation = AuthInvalidation {
        auth,
        usage: None,
        spend,
        transport: Arc::new(Silent),
        clock: Arc::new(SystemClock),
        access_group_ttl: 600.0_f64,
    };
    GatewayTeamCache {
        cache,
        invalidation: Arc::new(invalidation),
    }
}

#[tokio::test]
async fn gateway_cache_round_trip() {
    let cache = gateway();
    assert_eq!(cache.read_team("team_id:t1").await.unwrap(), None);
    let mut team = cached_team("t1");
    cache
        .write_team("team_alias:a1", &team, 30.0_f64)
        .await
        .unwrap();
    assert_eq!(
        cache.read_team("team_alias:a1").await.unwrap(),
        Some(team.clone())
    );
    cache.cache_team("t1", &mut team).await.unwrap();
    assert!(
        team.last_refreshed_at.is_some(),
        "_cache_team_object stamps the refresh"
    );
    assert_eq!(cache.read_team("team_id:t1").await.unwrap(), Some(team));

    let text = r#"{"access_group_id": "g1", "access_group_name": "group one"}"#;
    let group: AccessGroupTable = serde_json::from_str(text).unwrap();
    assert_eq!(
        cache.read_access_group("access_group_id:g1").await.unwrap(),
        None
    );
    cache.cache_access_group("g1", &group).await.unwrap();
    assert_eq!(
        cache.read_access_group("access_group_id:g1").await.unwrap(),
        Some(group)
    );
}
