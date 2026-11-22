use betterlite_db::spend_rows::key::END_USER_COUNTER_PREFIX;
use betterlite_db::spend_rows::rows::SpendCounterRows as DbSpendCounterRows;
use rust_decimal::Decimal;

use crate::spend_reseed::rows::SpendCounterRows;
use crate::spend_reseed::values::{SpendValueError, cache_amount};

use super::cache::{CounterBatch, CounterCache};
use super::{
    CurrentSpend, CurrentSpendError, FLOOR_MARKER_TTL_SECONDS, SpendQuery, SpendResult,
    WindowDescriptors,
};

pub(super) enum FloorCheck {
    // the request is decided on this spend without the strict check
    Settled(Decimal),
    Verified,
    Skipped,
}

// a window counter has no cheap staleness signal and strict mode trusts no stale-low counter, so both always recheck
pub(super) async fn recheck_floor<Cache, Batch, Rows>(
    spend: &CurrentSpend<'_, Cache, Batch, Rows>,
    query: &SpendQuery<'_>,
    current: Decimal,
) -> SpendResult<FloorCheck, Cache>
where
    Cache: CounterCache + ?Sized,
    Batch: CounterBatch + ?Sized,
    Rows: SpendCounterRows + DbSpendCounterRows,
{
    let higher_fallback = query.fallback_spend > current;
    if !higher_fallback && query.window.start.is_none() && !spend.fail_closed_budget_enforcement {
        return Ok(FloorCheck::Skipped);
    }
    match authoritative_floor_spend(spend, query.counter_key, &query.window).await? {
        Some(authoritative) if authoritative > current => {
            repair_stale_spend_counter(spend, query.counter_key, authoritative).await?;
            Ok(FloorCheck::Settled(authoritative))
        }
        Some(_) => Ok(FloorCheck::Verified),
        // tag counters, a failed read or no database: the caller's spend is the recorded value
        None if higher_fallback => Ok(FloorCheck::Settled(query.fallback_spend)),
        None => Ok(FloorCheck::Skipped),
    }
}

async fn authoritative_floor_spend<Cache, Batch, Rows>(
    spend: &CurrentSpend<'_, Cache, Batch, Rows>,
    counter_key: &str,
    window: &WindowDescriptors<'_>,
) -> SpendResult<Option<Decimal>, Cache>
where
    Cache: CounterCache + ?Sized,
    Batch: CounterBatch + ?Sized,
    Rows: SpendCounterRows + DbSpendCounterRows,
{
    let marker_key = format!("spend_db_floor:{counter_key}");
    if let Some(cached) = spend.local_amount(&marker_key).await? {
        return Ok(Some(cached));
    }
    let Some(db_spend) = floor_spend_from_db(spend, counter_key, window).await? else {
        return Ok(None);
    };
    // a reset that committed during the read wrote the post-reset floor, which beats this stale value
    if let Some(rechecked) = spend.local_amount(&marker_key).await? {
        return Ok(Some(rechecked));
    }
    spend
        .cache
        .memory_set_expiring(&marker_key, db_spend, FLOOR_MARKER_TTL_SECONDS)
        .map_err(CurrentSpendError::Cache)?;
    Ok(Some(db_spend))
}

async fn floor_spend_from_db<Cache, Batch, Rows>(
    spend: &CurrentSpend<'_, Cache, Batch, Rows>,
    counter_key: &str,
    window: &WindowDescriptors<'_>,
) -> Result<Option<Decimal>, SpendValueError>
where
    Cache: CounterCache + ?Sized,
    Batch: CounterBatch + ?Sized,
    Rows: SpendCounterRows + DbSpendCounterRows,
{
    let reader = spend.reseed.reader();
    if counter_key.starts_with(END_USER_COUNTER_PREFIX) {
        return Ok(reader.end_user_from_db(counter_key).await);
    }
    if let Some(entity_spend) = reader.from_db(counter_key).await {
        return Ok(Some(entity_spend));
    }
    let (Some(entity_type), Some(entity_id), Some(start)) =
        (window.entity_type, window.entity_id, window.start)
    else {
        return Ok(None);
    };
    spend
        .reseed
        .window_from_db(entity_type, entity_id, window.duration, start)
        .await
}

// monotonic: a slightly stale database total never lowers a counter a concurrent increment already raised
async fn repair_stale_spend_counter<Cache, Batch, Rows>(
    spend: &CurrentSpend<'_, Cache, Batch, Rows>,
    counter_key: &str,
    db_spend: Decimal,
) -> SpendResult<(), Cache>
where
    Cache: CounterCache + ?Sized,
    Batch: CounterBatch + ?Sized,
    Rows: SpendCounterRows,
{
    let cached = spend
        .cache
        .memory_get(counter_key)
        .await
        .map_err(CurrentSpendError::Cache)?;
    let high_enough = cached
        .as_ref()
        .is_some_and(|value| matches!(cache_amount(value), Ok(amount) if amount >= db_spend));
    if !high_enough {
        spend
            .cache
            .memory_set(counter_key, db_spend)
            .await
            .map_err(CurrentSpendError::Cache)?;
    }
    if !spend.cache.redis_configured() {
        return Ok(());
    }
    spend.batch.forget(counter_key);
    match spend.cache.redis_set_max(counter_key, db_spend).await {
        Ok(Some(repaired)) => spend.batch.record(counter_key, repaired),
        Ok(None) => {}
        Err(error) => {
            tracing::debug!(counter_key, error = ?error, "Unable to repair stale spend counter in Redis");
        }
    }
    Ok(())
}
