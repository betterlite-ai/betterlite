use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_models::decisions::Json;

use super::{DualCache, DualCacheError, value};
use crate::backend::memory::MemoryCacheOps;

pub(crate) fn memory_set<Memory>(
    cache: &DualCache<Memory>,
    key: &str,
    value: &Json,
    ttl: Option<f64>,
) -> Result<(), DualCacheError>
where
    Memory: MemoryCacheOps,
{
    let stored = value::cache_from_json(value)?;
    cache
        .in_memory_cache
        .set_cache(key, stored, ttl)
        .map_err(|source| DualCacheError::Memory { source })
}

// InMemoryCache.async_set_cache_sadd unions the new members into the stored set and rewrites it
pub(crate) fn add_to_memory_set<Memory>(
    cache: &DualCache<Memory>,
    key: &str,
    values: &[String],
    ttl: Option<f64>,
) -> Result<(), DualCacheError>
where
    Memory: MemoryCacheOps,
{
    let stored = cache
        .in_memory_cache
        .get_cache(key)
        .map_err(|source| DualCacheError::Memory { source })?;
    let mut members = match stored {
        None => Vec::new(),
        Some(CacheValue::Array(items)) => text_members(items, key)?,
        Some(_) => {
            return Err(DualCacheError::NotASet {
                key: key.to_owned(),
            });
        }
    };
    for candidate in values {
        if !members.iter().any(|member| member == candidate) {
            members.push(candidate.clone());
        }
    }
    let rewritten = CacheValue::Array(members.into_iter().map(CacheValue::Text).collect());
    cache
        .in_memory_cache
        .set_cache(key, rewritten, ttl)
        .map_err(|source| DualCacheError::Memory { source })
}

fn text_members(items: Vec<CacheValue>, key: &str) -> Result<Vec<String>, DualCacheError> {
    items
        .into_iter()
        .map(|item| match item {
            CacheValue::Text(text) => Ok(text),
            CacheValue::Null
            | CacheValue::Boolean(_)
            | CacheValue::Integer(_)
            | CacheValue::Float(_)
            | CacheValue::Array(_)
            | CacheValue::Object(_)
            | CacheValue::Bytes(_)
            | CacheValue::Credential(_) => Err(DualCacheError::NotASet {
                key: key.to_owned(),
            }),
        })
        .collect()
}

pub(crate) fn memory_set_pipeline<Memory>(
    cache: &DualCache<Memory>,
    entries: &[(String, Json)],
    ttl: Option<f64>,
) -> Result<(), DualCacheError>
where
    Memory: MemoryCacheOps,
{
    for (key, value) in entries {
        memory_set(cache, key, value, ttl)?;
    }
    Ok(())
}

pub(crate) fn redis_set<Memory>(
    cache: &DualCache<Memory>,
    key: &str,
    value: &Json,
    ttl: Option<f64>,
) where
    Memory: MemoryCacheOps,
{
    let Some(redis) = cache.redis() else {
        return;
    };
    redis.values.set_cache(key, value, ttl);
}

pub(crate) async fn async_redis_set<Memory>(
    cache: &DualCache<Memory>,
    key: &str,
    value: &Json,
    ttl: Option<f64>,
) where
    Memory: MemoryCacheOps,
{
    let Some(redis) = cache.redis() else {
        return;
    };
    if let Err(error) = redis
        .values
        .async_set_cache(Some(key), value, ttl, false)
        .await
    {
        tracing::error!(%error, "LiteLLM Cache: exception in async add_cache");
    }
}

pub(crate) async fn async_redis_set_pipeline<Memory>(
    cache: &DualCache<Memory>,
    entries: &[(String, Json)],
    ttl: Option<f64>,
) where
    Memory: MemoryCacheOps,
{
    let Some(redis) = cache.redis() else {
        return;
    };
    if let Err(error) = redis.values.async_set_cache_pipeline(entries, ttl).await {
        tracing::error!(%error, "LiteLLM Cache: exception in async add_cache");
    }
}
