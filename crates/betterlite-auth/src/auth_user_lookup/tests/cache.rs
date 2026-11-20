use std::sync::Arc;

use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCacheSettings, MAX_REDIS_BATCH_CACHE_SIZE};

use super::support::{model, user_with};
use crate::auth_cache::gateway_cache;
use crate::auth_user_lookup::cache::UserCache;

#[tokio::test]
async fn auth_cache_round_trips_and_reports_management_ttl() {
    let clock: Arc<dyn Fn() -> f64 + Send + Sync> = Arc::new(|| 1_000.0_f64);
    let memory = || {
        Arc::new(
            LocalMemoryOps::new(1024, Arc::clone(&clock)).expect("the local memory tier builds"),
        )
    };
    let settings = DualCacheSettings {
        default_in_memory_ttl: Some(60.0_f64),
        default_redis_ttl: None,
        batch_cache_expiry: None,
        max_redis_batch_cache_size: MAX_REDIS_BATCH_CACHE_SIZE,
        redis_delete_chunk_size: MAX_REDIS_BATCH_CACHE_SIZE,
    };
    let cache = gateway_cache(memory(), memory(), None, settings, 45.0_f64);

    assert_eq!(cache.management_ttl(), 60.0_f64);

    let key = "user_id:u1";
    assert!(cache.read(key).await.expect("the miss read").is_none());

    let user = model(&user_with("u1", Some("a@example.com"), None));
    cache
        .write(key, &user, cache.management_ttl())
        .await
        .expect("the user cache write");

    let cached = cache
        .read(key)
        .await
        .expect("the hit read")
        .expect("the cached user");
    assert_eq!(cached.user_id, "u1");
    assert_eq!(cached.user_email.as_deref(), Some("a@example.com"));
}
