use std::slice;

use betterlite_redis::request_batch::lock;
use rust_decimal::Decimal;

use super::adjust::{
    AdjustmentMode, InconsistentCounterPolicy, SettlementOptions, set_entries_actual_cost,
};
use super::counters::SettlementCounters;
use super::entry::ReservationEntry;
use super::error::SettlementError;
use super::handle::{ReservationHandle, get_reserved_counter_keys};
use super::reconcile::settle_reservation_to;

// release_budget_reservation: the request settled no cost, so the reservation comes all the way off the counters
pub async fn release_budget_reservation<C>(
    counters: &C,
    handle: &ReservationHandle,
) -> Result<(), SettlementError>
where
    C: SettlementCounters,
{
    settle_reservation_to(counters, handle, Decimal::ZERO).await?;
    Ok(())
}

// release_budget_reservation_on_cancel: a settle already in flight leaves the reservation alone, else a cancel settles at input_cost
pub async fn release_reservation_on_cancel<C>(
    counters: &C,
    handle: &ReservationHandle,
) -> Result<(), SettlementError>
where
    C: SettlementCounters,
{
    let finalized = lock(&handle.0).finalized;
    if finalized {
        return Ok(());
    }
    let input_cost = lock(&handle.0).input_cost;
    settle_reservation_to(counters, handle, input_cost).await?;
    Ok(())
}

// invalidate_budget_reservation_counters: a release that failed leaves the reserved counters dropped directly
pub async fn invalidate_reservation_keys<C>(counters: &C, handle: &ReservationHandle)
where
    C: SettlementCounters,
{
    let counter_keys = get_reserved_counter_keys(handle);
    for counter_key in counter_keys {
        counters.invalidate(&counter_key).await;
    }
}

// release_or_invalidate_budget_reservation: a terminal path that settles no cost releases, and a failed release drops the counters
pub async fn release_or_invalidate<C>(counters: &C, handle: &ReservationHandle)
where
    C: SettlementCounters,
{
    if lock(&handle.0).finalized {
        return;
    }
    let Err(error) = release_budget_reservation(counters, handle).await else {
        return;
    };
    tracing::error!(%error, "Failed to release budget reservation; invalidating counters");
    invalidate_reservation_keys(counters, handle).await;
    lock(&handle.0).finalized = true;
}

// release_unbound_budget_reservation: a reservation no logging callback owns releases here, since a bound one settles later
pub async fn release_unbound_reservation<C>(counters: &C, handle: &ReservationHandle)
where
    C: SettlementCounters,
{
    if !lock(&handle.0).callback_bound {
        release_or_invalidate(counters, handle).await;
    }
}

// _release_applied_entries_best_effort: a partial reservation rolls back one entry at a time, and a failed one is dropped directly
pub async fn release_entries_best_effort<C>(
    counters: &C,
    entries: &mut [ReservationEntry],
    default_reserved_cost: Decimal,
) where
    C: SettlementCounters,
{
    let options = SettlementOptions {
        inconsistent: InconsistentCounterPolicy::Reseed,
        mode: AdjustmentMode::Apply,
    };
    for entry in entries {
        let single = slice::from_mut(entry);
        let outcome = set_entries_actual_cost(
            counters,
            single,
            Decimal::ZERO,
            default_reserved_cost,
            options,
        )
        .await;
        let Err(error) = outcome else {
            continue;
        };
        tracing::error!(
            %error,
            "Failed to release partial budget reservation during exception cleanup"
        );
        let Some(counter_key) = &entry.counter_key else {
            continue;
        };
        counters.invalidate(counter_key).await;
    }
}
