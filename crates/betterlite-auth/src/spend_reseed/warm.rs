use rust_decimal::Decimal;

use crate::spend_reseed::cache::{ActiveSpendBatch, ReseedCache};

use super::reseed::{CoalescedError, CoalescedError::Cache as cache_failure};
use super::values::cache_amount;

// what Redis held when this pod's SET NX came back
pub(super) enum SeedAnswer {
    Seeded,
    Stored(Decimal),
    Vanished,
}

impl SeedAnswer {
    // the primary path trusts the seed whenever no other pod's value is readable
    pub(super) fn seeded(self, seed: Decimal) -> Decimal {
        match self {
            SeedAnswer::Stored(stored) => stored,
            SeedAnswer::Seeded | SeedAnswer::Vanished => seed,
        }
    }

    // the window path adds this pod's amount only when the winner vanished
    pub(super) fn held(self, seed: Decimal) -> Option<Decimal> {
        match self {
            SeedAnswer::Seeded => Some(seed),
            SeedAnswer::Stored(stored) => Some(stored),
            SeedAnswer::Vanished => None,
        }
    }
}

pub(super) async fn warm<Cache, Batch>(
    cache: &Cache,
    batch: &Batch,
    counter_key: &str,
    db_spend: Decimal,
    require_cache_warm: bool,
) -> Result<Option<Decimal>, CoalescedError<Cache::Error>>
where
    Cache: ReseedCache + ?Sized,
    Batch: ActiveSpendBatch + ?Sized,
{
    let mut current = db_spend;
    let result = if cache.redis_configured() {
        match redis_seed(cache, counter_key, db_spend).await {
            Ok(seed) => {
                current = seed.seeded(db_spend);
                remember(cache, batch, counter_key, current).await
            }
            Err(error) => Err(error),
        }
    } else {
        match memory_floor(cache, counter_key, db_spend).await {
            Ok(floor) => {
                current = floor;
                Ok(())
            }
            Err(error) => Err(error),
        }
    };
    if let Err(error) = result {
        tracing::error!(%error, counter_key, "SpendCounterReseed.coalesced: failed to warm the counter");
        if require_cache_warm {
            return Err(error);
        }
    }
    Ok(Some(current))
}

// SET NX, then learn what the key holds: this pod's seed, another pod's value, or nothing
pub(super) async fn redis_seed<Cache>(
    cache: &Cache,
    counter_key: &str,
    seed: Decimal,
) -> Result<SeedAnswer, CoalescedError<Cache::Error>>
where
    Cache: ReseedCache + ?Sized,
{
    let seeded = cache
        .redis_set_nx(counter_key, seed)
        .await
        .map_err(cache_failure)?;
    if seeded == Some(true) {
        return Ok(SeedAnswer::Seeded);
    }
    match cache.redis_get(counter_key).await.map_err(cache_failure)? {
        Some(value) => Ok(SeedAnswer::Stored(cache_amount(&value)?)),
        None => Ok(SeedAnswer::Vanished),
    }
}

// without Redis the local tier keeps the higher of the stored and the would-be seed amount
pub(super) async fn memory_floor<Cache>(
    tier: &Cache,
    key: &str,
    amount: Decimal,
) -> Result<Decimal, CoalescedError<Cache::Error>>
where
    Cache: ReseedCache + ?Sized,
{
    let mut floor = amount;
    if let Some(stored) = tier.memory_get(key).await.map_err(cache_failure)? {
        floor = amount.max(cache_amount(&stored)?);
    }
    tier.memory_set(key, floor).await.map_err(cache_failure)?;
    Ok(floor)
}

pub(super) async fn remember<Cache, Batch>(
    cache: &Cache,
    batch: &Batch,
    counter_key: &str,
    amount: Decimal,
) -> Result<(), CoalescedError<Cache::Error>>
where
    Cache: ReseedCache + ?Sized,
    Batch: ActiveSpendBatch + ?Sized,
{
    cache
        .memory_set(counter_key, amount)
        .await
        .map_err(cache_failure)?;
    batch.record(counter_key, amount);
    Ok(())
}
