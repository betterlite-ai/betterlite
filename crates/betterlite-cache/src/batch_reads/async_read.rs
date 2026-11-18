use betterlite_models::decisions::Json;

use super::apply::apply_batch_get;
use super::prepare::prepare_batch_get;
use super::reserve::rollback_redis_batch_key_reservations;
use super::value::redis_values;
use super::{BatchReadOptions, BatchReadsError};
use crate::backend::memory::MemoryCacheOps;
use crate::dual_cache::{DualCache, logged};

pub(super) async fn async_batch_get_cache<Memory>(
    cache: &DualCache<Memory>,
    keys: &[String],
    options: BatchReadOptions,
) -> Option<Vec<Option<Json>>>
where
    Memory: MemoryCacheOps,
{
    let message = "LiteLLM Cache: exception in async_batch_get_cache";
    logged(async_batch_read(cache, keys, options).await, message)
}

async fn async_batch_read<Memory>(
    cache: &DualCache<Memory>,
    keys: &[String],
    options: BatchReadOptions,
) -> Result<Vec<Option<Json>>, BatchReadsError>
where
    Memory: MemoryCacheOps,
{
    let pending = prepare_batch_get(cache, keys, options)?;
    if pending.redis_keys.is_empty() {
        return Ok(pending.result);
    }
    let Some(redis) = cache.redis() else {
        return Ok(pending.result);
    };
    let key_list: Vec<Option<&str>> = pending
        .redis_keys
        .iter()
        .map(|key| Some(key.as_str()))
        .collect();
    let values = match redis.async_batch_get_cache(&key_list).await {
        Ok(values) => values,
        Err(error) => {
            rollback_redis_batch_key_reservations(cache, &pending.previous_access_times);
            let failure = BatchReadsError::Redis(error);
            if !failure.circuit_open() {
                return Err(failure);
            }
            tracing::debug!(%failure, "LiteLLM Cache: async_batch_get_cache served from memory only");
            return Ok(pending.result);
        }
    };
    apply_batch_get(cache, &pending, Some(&redis_values(values)))
}
