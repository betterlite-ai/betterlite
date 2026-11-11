use betterlite_models::decisions::Json;

use super::{DualCache, DualCacheError, logged, value};
use crate::backend::memory::MemoryCacheOps;

pub(super) fn memory_set<Memory>(
    cache: &DualCache<Memory>,
    key: &str,
    value: &Json,
    ttl: Option<f64>,
) -> Option<()>
where
    Memory: MemoryCacheOps,
{
    let message = "LiteLLM Cache: exception in add_cache";
    let stored = logged(value::cache_from_json(value), message)?;
    logged(cache.in_memory_cache.set_cache(key, stored, ttl), message)
}

// InMemoryCache.async_set_cache_sadd unions the new members into the stored set and rewrites it
pub(super) fn add_to_memory_set<Memory>(
    cache: &DualCache<Memory>,
    key: &str,
    values: &[String],
    ttl: Option<f64>,
) -> Result<(), DualCacheError>
where
    Memory: MemoryCacheOps,
{
    let mut members: Vec<String> = match cache.in_memory_cache.get_cache(key)? {
        None => Vec::new(),
        Some(stored) => value::convert(&stored).map_err(|_source| DualCacheError::NotASet {
            key: key.to_owned(),
        })?,
    };
    for candidate in values {
        if !members.contains(candidate) {
            members.push(candidate.clone());
        }
    }
    let merged = value::convert(&members)?;
    cache.in_memory_cache.set_cache(key, merged, ttl)?;
    Ok(())
}
