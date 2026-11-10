use super::RedisCache;
use super::error::RedisCacheError;
use crate::cache_primitives::namespace::check_and_fix_namespace;
use crate::cache_primitives::pipeline_increment::IncrementOperation;

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

// pipeline_increment namespaces its own keys, so it gets them as the caller wrote them
pub(crate) async fn pipeline(
    cache: &RedisCache,
    increment_list: &[IncrementOperation],
) -> Result<Vec<f64>, RedisCacheError> {
    cache
        .counters
        .pipeline_increment(increment_list)
        .await
        .map_err(RedisCacheError::from)
}
