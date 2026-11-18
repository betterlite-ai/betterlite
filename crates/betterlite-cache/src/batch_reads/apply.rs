use std::collections::BTreeMap;

use betterlite_models::decisions::Json;

use super::BatchReadsError;
use super::PendingBatchRead;
use super::value::cache_from_json;
use crate::backend::memory::MemoryCacheOps;
use crate::dual_cache::{DualCache, lock_read};

// B4293's single read skips a backfill it cannot store and still serves the value; a batch read does the same
fn backfill<Memory>(
    cache: &DualCache<Memory>,
    key: &str,
    value: &Json,
) -> Result<(), BatchReadsError>
where
    Memory: MemoryCacheOps,
{
    let Some(stored) = cache_from_json(value) else {
        tracing::error!(key, "LiteLLM Cache: exception in get_cache");
        return Ok(());
    };
    let ttl = *lock_read(&cache.default_in_memory_ttl);
    cache
        .in_memory_cache
        .set_cache(key, stored, ttl)
        .map_err(BatchReadsError::Memory)
}

pub(super) fn backfill_values<Memory>(
    cache: &DualCache<Memory>,
    values: &BTreeMap<String, Option<Json>>,
) -> Result<(), BatchReadsError>
where
    Memory: MemoryCacheOps,
{
    for (key, value) in values {
        if let Some(present) = value {
            backfill(cache, key, present)?;
        }
    }
    Ok(())
}

// _apply_batch_get: a Redis value wins for every key the Redis result holds, the memory value stays otherwise
pub(super) fn apply_batch_get<Memory>(
    cache: &DualCache<Memory>,
    pending: &PendingBatchRead,
    redis_result: Option<&BTreeMap<String, Option<Json>>>,
) -> Result<Vec<Option<Json>>, BatchReadsError>
where
    Memory: MemoryCacheOps,
{
    let Some(values) = redis_result.filter(|values| values.values().any(Option::is_some)) else {
        return Ok(pending.result.clone());
    };
    backfill_values(cache, values)?;
    let merged = pending
        .keys
        .iter()
        .zip(&pending.result)
        .map(|(key, memory)| values.get(key).cloned().unwrap_or_else(|| memory.clone()));
    Ok(merged.collect())
}
