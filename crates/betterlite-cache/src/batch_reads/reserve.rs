use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_models::decisions::Json;

use super::BatchReadsError;
use super::prepare::memory_values;
use crate::backend::memory::MemoryCacheOps;
use crate::dual_cache::DualCache;

// one lock hold decides every due key, so two concurrent readers never fetch the same key
pub(super) fn reserve_redis_batch_keys<Memory>(
    cache: &DualCache<Memory>,
    current_time: f64,
    keys: &[String],
    result: &[Option<Json>],
) -> (Vec<String>, BTreeMap<String, Option<f64>>)
where
    Memory: MemoryCacheOps,
{
    let mut access_times = match cache.last_redis_batch_access_time.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let mut redis_keys: Vec<String> = Vec::new();
    let mut previous_access_times: BTreeMap<String, Option<f64>> = BTreeMap::new();
    let missing = keys.iter().zip(result).filter(|(_, value)| value.is_none());
    for (key, _) in missing {
        let due = access_times.entries.get(key).is_none_or(|recorded| {
            // float_arithmetic is denied; a fused multiply-add computes current_time - recorded
            let elapsed = recorded.mul_add(-1.0, current_time);
            elapsed >= cache.redis_batch_cache_expiry
        });
        if due {
            redis_keys.push(key.clone());
            previous_access_times.insert(key.clone(), access_times.entries.get(key).copied());
            access_times.insert(key.clone(), current_time);
        }
    }
    (redis_keys, previous_access_times)
}

pub(super) fn rollback_redis_batch_key_reservations<Memory>(
    cache: &DualCache<Memory>,
    previous_access_times: &BTreeMap<String, Option<f64>>,
) where
    Memory: MemoryCacheOps,
{
    let mut access_times = match cache.last_redis_batch_access_time.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    for (key, previous_time) in previous_access_times {
        match previous_time {
            Some(time) => access_times.insert(key.clone(), *time),
            None => {
                access_times.entries.shift_remove(key);
            }
        }
    }
}

pub(super) fn reserve_redis_batch_reads<Memory>(
    cache: &DualCache<Memory>,
    keys: &[String],
) -> (Vec<String>, BTreeMap<String, Option<f64>>)
where
    Memory: MemoryCacheOps,
{
    if cache.redis().is_none() {
        return (Vec::new(), BTreeMap::new());
    }
    let result = match memory_values(cache, keys) {
        Ok(result) => result,
        Err(error) => {
            tracing::error!(%error, "LiteLLM Cache: exception in reserve_redis_batch_reads");
            vec![None; keys.len()]
        }
    };
    let current_time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64());
    reserve_redis_batch_keys(cache, current_time, keys, &result)
}
