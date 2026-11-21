use std::sync::Arc;

use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCache, DualCacheSettings, MAX_REDIS_BATCH_CACHE_SIZE};
use betterlite_models::decisions::Json;

use super::support::{GROUP_A, TAG_A, group_budget, tag};
use crate::auth_cache::keys::CacheKey;
use crate::auth_cache::{AuthCache, gateway_cache};
use crate::auth_tag_budgets::cache::{CachedValue, TagBudgetCache as _};

type TestCache = AuthCache<DualCache<LocalMemoryOps>>;

const MANAGEMENT_TTL: f64 = 30.0;
const FALLBACK_TTL: f64 = 45.0;

fn local_cache() -> TestCache {
    let memory = || {
        Arc::new(
            LocalMemoryOps::new(1024, Arc::new(|| 0.0_f64)).expect("the local memory tier builds"),
        )
    };
    let settings = DualCacheSettings {
        default_in_memory_ttl: Some(MANAGEMENT_TTL),
        default_redis_ttl: None,
        batch_cache_expiry: None,
        max_redis_batch_cache_size: MAX_REDIS_BATCH_CACHE_SIZE,
        redis_delete_chunk_size: MAX_REDIS_BATCH_CACHE_SIZE,
    };
    gateway_cache(memory(), memory(), None, settings, FALLBACK_TTL)
}

#[tokio::test]
async fn auth_cache_tag_round_trip() {
    let cache = local_cache();
    let row = tag(TAG_A);
    let key = CacheKey::Tag(&row.tag_name).render();
    cache
        .write(&key, CachedValue::Tag(&row), MANAGEMENT_TTL)
        .await
        .expect("the tag write");
    let found = cache
        .read_tag(&key)
        .await
        .expect("the tag read")
        .expect("the stored tag");
    assert_eq!(found.tag_name, row.tag_name);
    assert_eq!(found.models, row.models);
    assert_eq!(found.spend, row.spend);
    assert!(
        cache
            .read_tag("tag:absent")
            .await
            .expect("the absent read")
            .is_none()
    );
}

#[tokio::test]
async fn auth_cache_group_round_trip() {
    let cache = local_cache();
    let budget = group_budget(GROUP_A);
    let key = CacheKey::ModelAccessGroup(&budget.access_group_name).render();
    cache
        .write(&key, CachedValue::Group(&budget), MANAGEMENT_TTL)
        .await
        .expect("the group write");
    assert_eq!(
        cache.read_group(&key).await.expect("the group read"),
        Some(budget)
    );
    assert!(
        cache
            .read_group("model_access_group:absent")
            .await
            .expect("the absent read")
            .is_none()
    );
}

#[tokio::test]
async fn auth_cache_registry_round_trip() {
    let cache = local_cache();
    let answer = Json::List(vec![Json::Text("tag-a".to_owned())]);
    let key = CacheKey::TagRegistry.render();
    cache
        .write(&key, CachedValue::Registry(&answer), MANAGEMENT_TTL)
        .await
        .expect("the registry write");
    assert_eq!(
        cache.read_registry(&key).await.expect("the registry read"),
        Some(answer)
    );
    assert!(
        cache
            .read_registry("tag_registry_absent")
            .await
            .expect("the absent read")
            .is_none()
    );
}

#[test]
fn auth_cache_management_ttl_is_used() {
    assert_eq!(
        local_cache().management_ttl().to_bits(),
        MANAGEMENT_TTL.to_bits()
    );
}
