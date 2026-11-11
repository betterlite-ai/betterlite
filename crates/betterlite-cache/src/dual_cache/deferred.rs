use std::sync::Arc;

use betterlite_models::decisions::Json;
use betterlite_redis::batch::RedisBatch;
use betterlite_redis::batch::result::BatchResult;

use super::{DualCache, DualCacheError, lock_read, writes};
use crate::backend::memory::MemoryCacheOps;

pub(crate) fn set_on_batch<Memory>(
    cache: &DualCache<Memory>,
    batch: &Arc<RedisBatch>,
    key: &str,
    value: &Json,
    ttl: Option<f64>,
) -> Result<Option<BatchResult<()>>, DualCacheError>
where
    Memory: MemoryCacheOps,
{
    let effective = ttl.or_else(|| *lock_read(&cache.default_in_memory_ttl));
    writes::memory_set(cache, key, value, effective)?;
    Ok(Some(batch.set(key, value, effective)))
}

pub(crate) fn delete_on_batch<Memory>(
    cache: &DualCache<Memory>,
    batch: &Arc<RedisBatch>,
    key: &str,
) -> Result<Option<BatchResult<()>>, DualCacheError>
where
    Memory: MemoryCacheOps,
{
    cache
        .in_memory_cache
        .delete_cache(key)
        .map_err(|source| DualCacheError::Memory { source })?;
    Ok(Some(batch.delete(key)))
}
