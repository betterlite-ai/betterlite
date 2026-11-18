use std::error::Error;
use std::future::Future;

use betterlite_db::database_failures::deadline::{DbLookupError, LookupRequest, bounded_db_lookup};
use betterlite_db::database_failures::gate::LoopBoundSemaphore;
use rust_decimal::Decimal;
use tokio_util::task::TaskTracker;

use super::reseed::{CoalescedError, CoalescedError::Cache as cache_failure};
use super::rows::{LookupAnswer, SpendCounterRows, SpendRow, SpendRowLookup};
use super::values::cache_amount;
use crate::spend_reseed::cache::{ActiveSpendBatch, ReseedCache};

pub(super) async fn gated_primary<Rows>(
    gate: &LoopBoundSemaphore,
    rows: &Rows,
    counter_key: &str,
) -> LookupAnswer<SpendRow, Rows::Error>
where
    Rows: SpendCounterRows,
{
    let _permit = match gate.current() {
        Some(semaphore) => semaphore.acquire_owned().await.ok(),
        None => None,
    };
    let Some(lookup) = primary_lookup(counter_key) else {
        return LookupAnswer::Row(None);
    };
    match rows.spend_row(lookup).await {
        Ok(row) => LookupAnswer::Row(row),
        Err(error) => LookupAnswer::Failed(error),
    }
}

// _counter_row's prefix order: key, team_member, team, user, org, project, and no end-user reseed
pub(super) fn primary_lookup(counter_key: &str) -> Option<SpendRowLookup<'_>> {
    if let Some(token) = counter_key.strip_prefix("spend:key:") {
        return Some(SpendRowLookup::Key(token));
    }
    if let Some(suffix) = counter_key.strip_prefix("spend:team_member:") {
        let (user_id, team_id) = suffix.rsplit_once(':')?;
        return Some(SpendRowLookup::TeamMember { user_id, team_id });
    }
    if let Some(team_id) = counter_key.strip_prefix("spend:team:") {
        return Some(SpendRowLookup::Team(team_id));
    }
    if let Some(user_id) = counter_key.strip_prefix("spend:user:") {
        return Some(SpendRowLookup::User(user_id));
    }
    if let Some(organization_id) = counter_key.strip_prefix("spend:org:") {
        return Some(SpendRowLookup::Organization(organization_id));
    }
    counter_key
        .strip_prefix("spend:project:")
        .map(SpendRowLookup::Project)
}

pub(super) async fn bounded_row<Failure, Lookup>(
    lookup: Lookup,
    request: LookupRequest<'_>,
    tasks: &TaskTracker,
    counter_key: &str,
) -> Option<Option<SpendRow>>
where
    Failure: Error + Send + Sync + 'static,
    Lookup:
        Future<Output = Result<LookupAnswer<SpendRow, Failure>, DbLookupError>> + Send + 'static,
{
    match bounded_db_lookup(lookup, request, tasks).await {
        Ok(LookupAnswer::Row(row)) => Some(row),
        Ok(LookupAnswer::Failed(error)) => {
            tracing::error!(%error, counter_key, "SpendCounterReseed: the counter row read failed");
            None
        }
        Err(error) => {
            tracing::error!(%error, counter_key, "SpendCounterReseed: the bounded lookup failed");
            None
        }
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
