#![cfg(test)]

use std::sync::Arc;

use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCache, DualCacheSettings, MAX_REDIS_BATCH_CACHE_SIZE};
use betterlite_models::decisions::Json;

use crate::agent_store::cache::ManagementObjectCache;
use crate::auth_cache::{AuthCache, gateway_cache};

type LocalCache = AuthCache<DualCache<LocalMemoryOps>>;

fn local_cache(memory_ttl: Option<f64>) -> LocalCache {
    let clock: Arc<dyn Fn() -> f64 + Send + Sync> = Arc::new(|| 1000.0_f64);
    let memory = Arc::new(LocalMemoryOps::new(1024, clock).expect("the local memory tier builds"));
    gateway_cache(
        Arc::clone(&memory),
        memory,
        None,
        DualCacheSettings {
            default_in_memory_ttl: memory_ttl,
            default_redis_ttl: None,
            batch_cache_expiry: None,
            max_redis_batch_cache_size: MAX_REDIS_BATCH_CACHE_SIZE,
            redis_delete_chunk_size: MAX_REDIS_BATCH_CACHE_SIZE,
        },
        45.0,
    )
}

#[tokio::test]
async fn cache_writes_and_reads_through_the_auth_cache() {
    let cache = local_cache(Some(30.0));
    let value = Json::Text(String::from("agent-1"));
    cache
        .write("agent_identity:key", &value, cache.management_ttl())
        .await;
    assert_eq!(cache.read("agent_identity:key").await, Some(value));
}

#[tokio::test]
async fn cache_management_ttl_uses_the_in_memory_ttl() {
    let cache = local_cache(Some(30.0));
    assert_eq!(cache.management_ttl().to_bits(), 30.0_f64.to_bits());
}
