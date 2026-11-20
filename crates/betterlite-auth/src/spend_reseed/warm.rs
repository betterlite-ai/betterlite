use rust_decimal::Decimal;

use crate::spend_reseed::cache::{ActiveSpendBatch, ReseedCache, WindowReseedCache};

use super::reseed::{CoalescedError, CoalescedError::Cache as cache_failure};
use super::values::cache_amount;

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
        warm_redis(cache, batch, counter_key, db_spend, &mut current).await
    } else {
        warm_memory(cache, counter_key, db_spend, &mut current).await
    };
    if let Err(error) = result {
        tracing::error!(%error, counter_key, "SpendCounterReseed.coalesced: failed to warm the counter");
        if require_cache_warm {
            return Err(error);
        }
    }
    Ok(Some(current))
}

async fn warm_redis<Cache, Batch>(
    cache: &Cache,
    batch: &Batch,
    counter_key: &str,
    db_spend: Decimal,
    current: &mut Decimal,
) -> Result<(), CoalescedError<Cache::Error>>
where
    Cache: ReseedCache + ?Sized,
    Batch: ActiveSpendBatch + ?Sized,
{
    let seeded = cache
        .redis_set_nx(counter_key, db_spend)
        .await
        .map_err(cache_failure)?;
    if seeded != Some(true) {
        let stored = cache.redis_get(counter_key).await.map_err(cache_failure)?;
        *current = match stored {
            Some(value) => cache_amount(&value)?,
            None => db_spend,
        };
    }
    cache
        .memory_set(counter_key, *current)
        .await
        .map_err(cache_failure)?;
    batch.record(counter_key, *current);
    Ok(())
}

async fn warm_memory<Cache>(
    cache: &Cache,
    counter_key: &str,
    db_spend: Decimal,
    current: &mut Decimal,
) -> Result<(), CoalescedError<Cache::Error>>
where
    Cache: ReseedCache + ?Sized,
{
    let stored = cache.memory_get(counter_key).await.map_err(cache_failure)?;
    let seeded = match stored {
        Some(value) => db_spend.max(cache_amount(&value)?),
        None => db_spend,
    };
    if let Err(error) = cache.memory_set(counter_key, seeded).await {
        return Err(cache_failure(error));
    }
    *current = seeded;
    Ok(())
}

// coalesced_window's warm contract differs from the primary one: a lost SET NX whose winner
// vanished adds the window amount instead of reseeding it, and every failure propagates
pub(super) async fn warm_window<Cache, Batch>(
    cache: &Cache,
    batch: &Batch,
    counter_key: &str,
    window_spend: Decimal,
) -> Result<Option<Decimal>, CoalescedError<Cache::Error>>
where
    Cache: WindowReseedCache + ?Sized,
    Batch: ActiveSpendBatch + ?Sized,
{
    let result = if cache.redis_configured() {
        warm_window_redis(cache, batch, counter_key, window_spend).await
    } else {
        warm_window_memory(cache, counter_key, window_spend).await
    };
    match result {
        Ok(current) => Ok(Some(current)),
        Err(error) => {
            tracing::error!(
                %error,
                counter_key,
                "SpendCounterReseed.coalesced_window: failed to warm the counter"
            );
            Err(error)
        }
    }
}

async fn warm_window_redis<Cache, Batch>(
    cache: &Cache,
    batch: &Batch,
    counter_key: &str,
    window_spend: Decimal,
) -> Result<Decimal, CoalescedError<Cache::Error>>
where
    Cache: WindowReseedCache + ?Sized,
    Batch: ActiveSpendBatch + ?Sized,
{
    let seeded = cache
        .redis_set_nx(counter_key, window_spend)
        .await
        .map_err(cache_failure)?;
    let current = if seeded == Some(true) {
        window_spend
    } else {
        match cache.redis_get(counter_key).await.map_err(cache_failure)? {
            Some(value) => cache_amount(&value)?,
            // the winner expired between SET NX and the read: add this pod's copy instead of reseeding
            None => cache
                .redis_increment(counter_key, window_spend)
                .await
                .map_err(cache_failure)?,
        }
    };
    cache
        .memory_set(counter_key, current)
        .await
        .map_err(cache_failure)?;
    batch.record(counter_key, current);
    Ok(current)
}

async fn warm_window_memory<Cache>(
    cache: &Cache,
    counter_key: &str,
    window_spend: Decimal,
) -> Result<Decimal, CoalescedError<Cache::Error>>
where
    Cache: WindowReseedCache + ?Sized,
{
    let stored = cache.memory_get(counter_key).await.map_err(cache_failure)?;
    let seeded = match stored {
        Some(value) => window_spend.max(cache_amount(&value)?),
        None => window_spend,
    };
    if let Err(error) = cache.memory_set(counter_key, seeded).await {
        return Err(cache_failure(error));
    }
    Ok(seeded)
}
