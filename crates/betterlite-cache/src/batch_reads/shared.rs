use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_models::decisions::Json;
use betterlite_redis::redis_cache::RedisCache;

use super::apply::apply_batch_get;
use super::async_read::async_batch_get_cache;
use super::prepare::prepare_batch_get;
use super::reserve::rollback_redis_batch_keys;
use super::value::redis_values;
use super::{BatchReadOptions, BatchReadsError, PendingBatchRead};
use crate::backend::memory::MemoryCacheOps;
use crate::dual_cache::DualCache;

// async_batch_get_cache's own defaults: memory and every throttle still apply per cache
const SHARED: BatchReadOptions = BatchReadOptions {
    local_only: false,
    throttle_redis: true,
};

type SharedSlots = BTreeMap<usize, Option<Vec<Option<Json>>>>;

pub(super) async fn async_batch_get_cache_shared<Memory>(
    reads: &[(&DualCache<Memory>, &[String])],
) -> Vec<Option<Vec<Option<Json>>>>
where
    Memory: MemoryCacheOps,
{
    let shared_redis = reads.first().and_then(|(cache, _keys)| cache.redis());
    let mut slots: SharedSlots = BTreeMap::new();
    let mut prepared: Vec<(usize, &DualCache<Memory>, PendingBatchRead)> = Vec::new();
    for (index, (cache, keys)) in reads.iter().enumerate() {
        if !shares_client(cache, shared_redis.as_ref()) {
            slots.insert(index, async_batch_get_cache(cache, keys, SHARED).await);
            continue;
        }
        match prepare_batch_get(cache, keys, SHARED) {
            Ok(pending) => {
                slots.insert(index, Some(pending.result.clone()));
                prepared.push((index, *cache, pending));
            }
            Err(error) => {
                tracing::error!(%error, "LiteLLM Cache: exception in async_batch_get_cache_shared");
            }
        }
    }
    if let Some(redis) = shared_redis.as_ref()
        && !prepared.is_empty()
    {
        read_and_apply(redis, &prepared, &mut slots).await;
    }
    (0..reads.len())
        .map(|index| slots.remove(&index).flatten())
        .collect()
}

// LiteLLM compares the Redis clients by identity: only the first pair's client carries the shared read
fn shares_client<Memory>(cache: &DualCache<Memory>, shared_redis: Option<&Arc<RedisCache>>) -> bool
where
    Memory: MemoryCacheOps,
{
    match (shared_redis, cache.redis()) {
        (Some(shared), Some(own)) => Arc::ptr_eq(shared, &own),
        _ => false,
    }
}

// dict.fromkeys over every prepared cache's keys: first-seen order, no duplicates
fn merged_keys<Memory>(prepared: &[(usize, &DualCache<Memory>, PendingBatchRead)]) -> Vec<String>
where
    Memory: MemoryCacheOps,
{
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut keys: Vec<String> = Vec::new();
    for (_, _, pending) in prepared {
        for key in &pending.redis_keys {
            if seen.insert(key.clone()) {
                keys.push(key.clone());
            }
        }
    }
    keys
}

async fn read_and_apply<Memory>(
    redis: &Arc<RedisCache>,
    prepared: &[(usize, &DualCache<Memory>, PendingBatchRead)],
    slots: &mut SharedSlots,
) where
    Memory: MemoryCacheOps,
{
    let keys = merged_keys(prepared);
    if keys.is_empty() {
        return;
    }
    let key_list: Vec<Option<&str>> = keys.iter().map(|key| Some(key.as_str())).collect();
    let values = match redis.async_batch_get_cache(&key_list).await {
        Ok(values) => values,
        Err(error) => {
            let failure = BatchReadsError::Redis(error);
            shared_read_failure(&failure, prepared, slots);
            return;
        }
    };
    let converted = redis_values(&values);
    for (index, cache, pending) in prepared {
        let own: BTreeMap<String, Option<Json>> = pending
            .redis_keys
            .iter()
            .filter_map(|key| converted.get(key).map(|value| (key.clone(), value.clone())))
            .collect();
        match apply_batch_get(cache, pending, Some(&own)) {
            Ok(merged) => {
                slots.insert(*index, Some(merged));
            }
            Err(error) => {
                tracing::error!(%error, "LiteLLM Cache: exception in async_batch_get_cache_shared");
                slots.insert(*index, None);
            }
        }
    }
}

pub(super) fn shared_read_failure<Memory>(
    error: &BatchReadsError,
    prepared: &[(usize, &DualCache<Memory>, PendingBatchRead)],
    slots: &mut SharedSlots,
) where
    Memory: MemoryCacheOps,
{
    let circuit_open = error.circuit_open();
    for (index, cache, pending) in prepared {
        rollback_redis_batch_keys(cache, &pending.previous_access_times);
        if !circuit_open && !pending.redis_keys.is_empty() {
            slots.insert(*index, None);
        }
    }
    if circuit_open {
        tracing::debug!(%error, "LiteLLM Cache: async_batch_get_cache_shared served from memory only");
    } else {
        tracing::error!(%error, "LiteLLM Cache: exception in async_batch_get_cache_shared");
    }
}
