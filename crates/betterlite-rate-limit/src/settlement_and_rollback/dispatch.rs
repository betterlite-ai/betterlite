use std::time::UNIX_EPOCH;

use crate::atomic_counter::in_memory::{
    LocalCounterCache,
    // ast-grep-ignore: no-long-names
    atomic_check_and_increment_in_memory as check_in_memory,
};
use crate::limiter_records::{
    AtomicCounterMeta, CacheCounterValue, DescriptorAtomicGroup, RateLimitResponse,
};
use crate::rate_limit_window_store::CounterCache;

use super::lua_per_descriptor::lua_per_descriptor;
use super::{AtomicSettlement, SettlementError};

mod tests;

pub(super) async fn dispatch(
    settlement: &AtomicSettlement<'_>,
    groups: &[DescriptorAtomicGroup],
) -> Result<RateLimitResponse, SettlementError> {
    if let Some(script) = settlement.script.as_ref() {
        return lua_per_descriptor(settlement, groups, script).await;
    }
    settle_in_memory(settlement, groups).await
}

pub(super) async fn settle_in_memory(
    settlement: &AtomicSettlement<'_>,
    groups: &[DescriptorAtomicGroup],
) -> Result<RateLimitResponse, SettlementError> {
    let _guard = settlement.windows.check_and_increment_lock.lock().await;
    let now = settlement_seconds(settlement)?;
    let flat_meta: Vec<AtomicCounterMeta> = groups
        .iter()
        .flat_map(|(_keys, _args, meta)| meta.iter().cloned())
        .collect();
    let cache = WindowCacheAdapter {
        cache: settlement.windows.cache.as_ref(),
    };
    Ok(check_in_memory(&cache, &flat_meta, now).await?)
}

fn settlement_seconds(settlement: &AtomicSettlement<'_>) -> Result<i64, SettlementError> {
    let elapsed = settlement
        .clock
        .current_time()
        .duration_since(UNIX_EPOCH)
        .map_err(SettlementError::ClockBeforeEpoch)?;
    let seconds = elapsed.as_secs();
    i64::try_from(seconds).map_err(|source| SettlementError::ClockOverflow {
        value: seconds,
        source,
    })
}

// LiteLLM's in-memory helpers read its internal usage cache, the same store this crate reads counters from
struct WindowCacheAdapter<'cache> {
    cache: &'cache dyn CounterCache,
}

#[async_trait::async_trait]
impl LocalCounterCache for WindowCacheAdapter<'_> {
    async fn get(&self, key: &str, local_only: bool) -> Option<CacheCounterValue> {
        self.cache.get_cache(key, local_only).await
    }

    async fn set(&self, key: &str, value: CacheCounterValue, ttl: i64, local_only: bool) {
        self.cache.set_cache(key, value, ttl, local_only).await;
    }
}
