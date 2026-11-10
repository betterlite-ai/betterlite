use crate::redis_cache::RedisCache;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct BackendKey {
    pub(super) cache_key: String,
    pub(super) namespace: Option<String>,
}

// litellm.caching.redis_batch._backend_key: two caches built from the same connection settings and namespace share one pipeline, and the settings already carry the family, so the client cache key stands in for the sorted kwargs dict.
pub(crate) fn backend_key(redis_cache: &RedisCache) -> BackendKey {
    BackendKey {
        cache_key: redis_cache.transport.cache_key.clone(),
        namespace: redis_cache.namespace.clone(),
    }
}
