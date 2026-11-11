use std::sync::Arc;

use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_models::decisions::Json;
use betterlite_redis::redis_cache::RedisCache;

use super::{DualCacheError, value};
use crate::backend::memory::MemoryCacheOps;

pub(crate) fn memory_set(
    memory: &dyn MemoryCacheOps,
    key: &str,
    value: &Json,
    ttl: Option<f64>,
) -> Result<(), DualCacheError> {
    let stored = value::cache_from_json(value)?;
    memory
        .set_cache(key, stored, ttl)
        .map_err(|source| DualCacheError::Memory { source })
}

pub(crate) fn redis_set(
    redis: Option<&Arc<RedisCache>>,
    key: &str,
    value: &Json,
    ttl: Option<f64>,
) {
    if let Some(tier) = redis {
        tier.values.set_cache(key, value, ttl);
    }
}

pub(crate) async fn async_redis_set(
    redis: Option<&Arc<RedisCache>>,
    key: &str,
    value: &Json,
    ttl: Option<f64>,
) {
    let Some(tier) = redis else {
        return;
    };
    if let Err(error) = tier
        .values
        .async_set_cache(Some(key), value, ttl, false)
        .await
    {
        tracing::error!(%error, "LiteLLM Cache: exception in async add_cache");
    }
}

pub(crate) fn memory_set_pipeline(
    memory: &dyn MemoryCacheOps,
    entries: &[(String, Json)],
    ttl: Option<f64>,
) -> Result<(), DualCacheError> {
    for (key, value) in entries {
        memory_set(memory, key, value, ttl)?;
    }
    Ok(())
}

pub(crate) async fn async_redis_set_pipeline(
    redis: Option<&Arc<RedisCache>>,
    entries: &[(String, Json)],
    ttl: Option<f64>,
) {
    let Some(tier) = redis else {
        return;
    };
    if let Err(error) = tier.values.async_set_cache_pipeline(entries, ttl).await {
        tracing::error!(%error, "LiteLLM Cache: exception in async add_cache");
    }
}

// InMemoryCache.async_set_cache_sadd unions the new members into the stored set and rewrites it
pub(crate) fn add_to_memory_set(
    memory: &dyn MemoryCacheOps,
    key: &str,
    values: &[String],
    ttl: Option<f64>,
) -> Result<(), DualCacheError> {
    let stored = memory
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
    memory
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
