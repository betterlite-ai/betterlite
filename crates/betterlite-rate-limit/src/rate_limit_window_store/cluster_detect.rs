use super::{CounterCache, RedisCacheKind};

#[must_use]
pub fn is_redis_cluster(cache: &dyn CounterCache) -> bool {
    if let Some(RedisCacheKind::Cluster) = cache.redis_cache_kind() {
        return true;
    }
    false
}

mod tests;
