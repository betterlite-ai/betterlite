use std::error::Error;

use betterlite_db::budget_permission_rows::Timestamp;
use rust_decimal::Decimal;

use crate::budget_counter::CounterEntityType;
use crate::spend_reseed::cache::{ActiveSpendBatch, WindowReseedCache};

use super::reseed::{CoalescedError, CoalescedError::Cache as cache_failure};
use super::warm::{memory_floor, redis_seed, remember};

pub struct WindowQuery<'entity> {
    pub entity_type: &'entity CounterEntityType,
    pub entity_id: &'entity str,
    pub start: Timestamp,
}

// everything coalesced_window reads before it warms: the counter, the window and that window's duration
pub struct WindowReseed<'entity> {
    pub counter_key: &'entity str,
    pub entity_type: &'entity CounterEntityType,
    pub entity_id: &'entity str,
    pub window_duration: Option<&'entity str>,
    pub window_start: Timestamp,
}

// a lost SET NX with a vanished winner adds the window amount, and window warm failures propagate
pub(super) async fn warm_window<Cache, Batch>(
    cache: &Cache,
    batch: &Batch,
    key: &str,
    spent: Decimal,
) -> Result<Option<Decimal>, CoalescedError<Cache::Error>>
where
    Cache: WindowReseedCache + ?Sized,
    Batch: ActiveSpendBatch + ?Sized,
{
    if !cache.redis_configured() {
        return memory_floor(cache, key, spent)
            .await
            .map(Some)
            .map_err(|error| window_failure(error, key));
    }
    let answer = window_seed(cache, key, spent)
        .await
        .map_err(|error| window_failure(error, key))?;
    remember(cache, batch, key, answer)
        .await
        .map(|()| Some(answer))
        .map_err(|error| window_failure(error, key))
}

// a lost SET NX with a vanished winner adds this pod's window amount instead of reseeding it
async fn window_seed<Cache>(
    cache: &Cache,
    key: &str,
    spent: Decimal,
) -> Result<Decimal, CoalescedError<Cache::Error>>
where
    Cache: WindowReseedCache + ?Sized,
{
    match redis_seed(cache, key, spent).await?.held(spent) {
        Some(answer) => Ok(answer),
        None => cache
            .redis_increment(key, spent)
            .await
            .map_err(cache_failure),
    }
}

fn window_failure<CacheError>(
    error: CoalescedError<CacheError>,
    counter_key: &str,
) -> CoalescedError<CacheError>
where
    CacheError: Error,
{
    tracing::error!(
        %error,
        counter_key,
        "SpendCounterReseed.coalesced_window: failed to warm the counter"
    );
    error
}
