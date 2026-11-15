use super::{CounterCache, RedisCacheKind};

#[must_use]
pub fn is_redis_cluster(cache: &dyn CounterCache) -> bool {
    match cache.redis_cache_kind() {
        Some(RedisCacheKind::Cluster) => true,
        Some(RedisCacheKind::Standalone) | None => false,
    }
}

mod tests;
