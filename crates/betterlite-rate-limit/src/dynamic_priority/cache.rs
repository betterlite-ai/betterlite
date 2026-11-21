use crate::limiter_records::CacheCounterValue;

mod tests;

// DualCache.async_get_cache as a saturation read uses it: the caller's local TTL, and a miss that is not an error
#[async_trait::async_trait]
pub trait SaturationCache: Send + Sync {
    async fn get_cache(
        &self,
        key: &str,
        local_only: bool,
        local_ttl: i64,
    ) -> Option<CacheCounterValue>;
}

// _get_saturation_value_from_cache: reads the counter through the shared usage cache, which reaches Redis because local_only stays false
#[must_use]
// ast-grep-ignore: no-one-line-functions
pub async fn saturation_value_from_cache(
    cache: &dyn SaturationCache,
    counter_key: &str,
    local_ttl: i64,
) -> Option<CacheCounterValue> {
    cache.get_cache(counter_key, false, local_ttl).await
}
