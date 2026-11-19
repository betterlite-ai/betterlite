use std::sync::Arc;

use betterlite_auth::auth_cache::AuthCache;
use betterlite_auth::auth_cache::gateway_cache;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCache, DualCacheSettings, MAX_REDIS_BATCH_CACHE_SIZE};

// A memory-only B733 cache: no Redis, so a set, read and delete stay on the local tier
pub(crate) fn memory_auth_cache() -> AuthCache<DualCache<LocalMemoryOps>> {
    let clock: Arc<dyn Fn() -> f64 + Send + Sync> = Arc::new(|| 1_000.0_f64);
    let ordinary =
        Arc::new(LocalMemoryOps::new(1024, Arc::clone(&clock)).expect("the ordinary tier builds"));
    let key_objects = Arc::new(LocalMemoryOps::new(1024, clock).expect("the key tier builds"));
    gateway_cache(
        ordinary,
        key_objects,
        None,
        DualCacheSettings {
            default_in_memory_ttl: Some(60.0_f64),
            default_redis_ttl: None,
            batch_cache_expiry: None,
            max_redis_batch_cache_size: MAX_REDIS_BATCH_CACHE_SIZE,
            redis_delete_chunk_size: MAX_REDIS_BATCH_CACHE_SIZE,
        },
        45.0_f64,
    )
}
