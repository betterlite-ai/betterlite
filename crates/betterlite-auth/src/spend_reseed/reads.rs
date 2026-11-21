use std::sync::Arc;

use betterlite_db::spend_rows::key::WindowDurationValidity;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::budget_primitives::window_start::DurationParser;
use crate::spend_reseed::cache::ReseedCache;

use super::reseed::{CoalescedError, CoalescedError::Cache as cache_failure};
use super::values::cache_amount;

// B199's duration parser decides which suffixes make a key/team counter a window counter
pub(super) struct CounterDurations {
    pub(super) durations: Arc<dyn DurationParser + Send + Sync>,
    pub(super) now: DateTime<Utc>,
}

impl WindowDurationValidity for CounterDurations {
    fn is_valid(&self, spec: &str) -> bool {
        self.durations.seconds(spec, self.now).is_some()
    }
}

// the re-check after the lock: a batch answer is an authoritative Redis miss, so local memory is stale
pub(super) async fn cached_amount<Cache>(
    cache: &Cache,
    batch_miss: bool,
    counter_key: &str,
) -> Result<Option<Decimal>, CoalescedError<Cache::Error>>
where
    Cache: ReseedCache + ?Sized,
{
    let mut clean_miss = batch_miss;
    if cache.redis_configured() && !clean_miss {
        match cache.redis_get(counter_key).await {
            Ok(Some(value)) => {
                if let Ok(amount) = cache_amount(&value) {
                    return Ok(Some(amount));
                }
            }
            Ok(None) => clean_miss = true,
            // a failed Redis read silently leaves the per-pod cache as the only fallback
            Err(_error) => {}
        }
    }
    if !clean_miss
        && let Some(value) = cache.memory_get(counter_key).await.map_err(cache_failure)?
    {
        return Ok(Some(cache_amount(&value)?));
    }
    Ok(None)
}
