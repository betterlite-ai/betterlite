use super::RedisCache;
use super::error::RedisCacheError;
use crate::cache_primitives::namespace::check_and_fix_namespace;

// incr_by_float_with_ttl takes an already namespaced key
pub(crate) async fn increment(
    cache: &RedisCache,
    key: &str,
    value: f64,
    ttl: i64,
    refresh_ttl: bool,
) -> Result<f64, RedisCacheError> {
    let namespaced = check_and_fix_namespace(cache.namespace.as_deref(), Some(key))
        .unwrap_or_else(|| key.to_owned());
    cache
        .counters
        .incr_by_float_with_ttl(&namespaced, value, Some(ttl), refresh_ttl)
        .await
        .map_err(RedisCacheError::from)
}
