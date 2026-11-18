use std::sync::Arc;

use betterlite_models::decisions::Json;
use betterlite_redis::batch::RedisBatch;

use super::apply::apply_batch_get;
use super::prepare::prepare_batch_get;
use super::value::redis_map_values;
use super::{BatchReadOptions, BatchReadsError, DeclaredBatchRead};
use crate::backend::memory::MemoryCacheOps;
use crate::dual_cache::DualCache;

// B4331's request pipeline already de-duplicates the round trip, so a declared read takes every missing key
const DECLARED: BatchReadOptions = BatchReadOptions {
    local_only: false,
    throttle_redis: false,
};

pub(super) fn declare_batch_get<Memory>(
    cache: &DualCache<Memory>,
    keys: &[String],
    batch: &Arc<RedisBatch>,
) -> Result<DeclaredBatchRead, BatchReadsError>
where
    Memory: MemoryCacheOps,
{
    let pending = prepare_batch_get(cache, keys, DECLARED)?;
    let result = if pending.redis_keys.is_empty() {
        None
    } else {
        Some(batch.mget(&pending.redis_keys))
    };
    Ok(DeclaredBatchRead {
        keys: keys.to_vec(),
        pending,
        result,
    })
}

pub(super) async fn async_resolve_batch_get<Memory>(
    cache: &DualCache<Memory>,
    declared: DeclaredBatchRead,
) -> Result<Vec<Option<Json>>, BatchReadsError>
where
    Memory: MemoryCacheOps,
{
    let Some(result) = declared.result else {
        return apply_batch_get(cache, &declared.pending, None);
    };
    let fetched = result.await.map_err(BatchReadsError::Batch)?;
    let values = redis_map_values(&fetched);
    apply_batch_get(cache, &declared.pending, Some(&values))
}
