use std::sync::Arc;

use betterlite_models::decisions::Json;
use betterlite_redis::batch::RedisBatch;
use betterlite_redis::batch::result::BatchResult;

use super::{DualCacheError, writes};
use crate::backend::memory::MemoryCacheOps;

pub(crate) fn set_on_batch(
    memory: &dyn MemoryCacheOps,
    batch: &Arc<RedisBatch>,
    key: &str,
    value: &Json,
    ttl: Option<f64>,
) -> Result<Option<BatchResult<()>>, DualCacheError> {
    writes::memory_set(memory, key, value, ttl)?;
    Ok(Some(batch.set(key, value, ttl)))
}

pub(crate) fn delete_on_batch(
    memory: &dyn MemoryCacheOps,
    batch: &Arc<RedisBatch>,
    key: &str,
) -> Result<Option<BatchResult<()>>, DualCacheError> {
    memory
        .delete_cache(key)
        .map_err(|source| DualCacheError::Memory { source })?;
    Ok(Some(batch.delete(key)))
}
