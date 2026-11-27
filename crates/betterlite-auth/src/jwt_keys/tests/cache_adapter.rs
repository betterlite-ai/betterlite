use std::sync::Arc;

use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::disk::Clock as DiskClock;
use betterlite_cache::dual_cache::{DualCacheSettings, MAX_REDIS_BATCH_CACHE_SIZE};

use super::support::{FakeClock, key};
use crate::auth_cache::gateway_cache;
use crate::jwt_keys::cache::KeyCache as _;
use crate::jwt_keys::clock::Clock as _;

#[tokio::test]
async fn auth_cache_adapter_preserves_values_and_ttl() {
    let clock = Arc::new(FakeClock::new(1000.0));
    let wall_clock = Arc::clone(&clock);
    let backend_clock: DiskClock = Arc::new(move || wall_clock.now());
    let ordinary = Arc::new(LocalMemoryOps::new(1024, Arc::clone(&backend_clock)).unwrap());
    let key_objects = Arc::new(LocalMemoryOps::new(1024, backend_clock).unwrap());
    let cache = gateway_cache(
        ordinary,
        key_objects,
        None,
        DualCacheSettings {
            default_in_memory_ttl: None,
            default_redis_ttl: None,
            batch_cache_expiry: None,
            max_redis_batch_cache_size: MAX_REDIS_BATCH_CACHE_SIZE,
            redis_delete_chunk_size: MAX_REDIS_BATCH_CACHE_SIZE,
        },
        0.0,
    );
    assert_eq!(cache.read_value("jwks").await.unwrap(), None);
    cache.write_value("jwks", &key("a"), 10.0).await.unwrap();
    assert_eq!(cache.read_value("jwks").await.unwrap(), Some(key("a")));
    clock.advance(9.0);
    assert_eq!(cache.read_value("jwks").await.unwrap(), Some(key("a")));
    clock.advance(2.0);
    assert_eq!(cache.read_value("jwks").await.unwrap(), None);
}
