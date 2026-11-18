use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_models::decisions::Json;

use super::reserve::reserve_redis_batch_keys;
use super::{BatchReadOptions, BatchReadsError, PendingBatchRead};
use crate::backend::memory::MemoryCacheOps;
use crate::dual_cache::DualCache;
use crate::handler::value::to_json;

// InMemoryCache answers None for a missing key and for a stored None; both read as a miss
pub(super) fn memory_values<Memory>(
    cache: &DualCache<Memory>,
    keys: &[String],
) -> Result<Vec<Option<Json>>, BatchReadsError>
where
    Memory: MemoryCacheOps,
{
    let stored = cache
        .in_memory_cache
        .batch_get_cache(keys)
        .map_err(BatchReadsError::Memory)?;
    stored
        .into_iter()
        .map(|value| match value {
            None | Some(CacheValue::Null) => Ok(None),
            Some(stored) => to_json(&stored)
                .map(Some)
                .ok_or(BatchReadsError::UnrepresentableValue),
        })
        .collect()
}

pub(super) fn prepare_batch_get<Memory>(
    cache: &DualCache<Memory>,
    keys: &[String],
    options: BatchReadOptions,
) -> Result<PendingBatchRead, BatchReadsError>
where
    Memory: MemoryCacheOps,
{
    let result = memory_values(cache, keys)?;
    let mut redis_keys: Vec<String> = Vec::new();
    let mut previous_access_times: BTreeMap<String, Option<f64>> = BTreeMap::new();
    if cache.redis().is_some() && !options.local_only {
        if options.throttle_redis {
            // LiteLLM reserves against time.time(), the wall clock in epoch seconds
            let current_time = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0.0, |elapsed| elapsed.as_secs_f64());
            (redis_keys, previous_access_times) =
                reserve_redis_batch_keys(cache, current_time, keys, &result);
        } else {
            redis_keys = keys
                .iter()
                .zip(&result)
                .filter(|(_, value)| value.is_none())
                .map(|(key, _)| key.clone())
                .collect();
        }
    }
    Ok(PendingBatchRead {
        keys: keys.to_vec(),
        result,
        redis_keys,
        previous_access_times,
    })
}
