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
use super::reconcile::{ReservationFinality, reconcile_budget_reservation};

// release_budget_reservation: the request settled no cost, so the reservation comes all the way off the counters
pub async fn release_budget_reservation<C>(
    counters: &C,
    handle: &ReservationHandle,
) -> Result<(), SettlementError>
where
    C: SettlementCounters,
{
    let _pending = reconcile_budget_reservation(
        counters,
        handle,
        Decimal::ZERO,
        ReservationFinality::Finalize,
        AdjustmentMode::Apply,
    )
    .await?;
    Ok(())
}

// release_budget_reservation_on_cancel: the provider already billed the input tokens, so a cancel settles at input_cost
// The caller spawns this on a server-owned task, betterlite's stand-in for asyncio.shield, so a cancelled request cannot drop it
pub async fn release_reservation_on_cancel<C>(
    counters: &C,
    handle: &ReservationHandle,
) -> Result<(), SettlementError>
where
    C: SettlementCounters,
{
    let input_cost = lock(&handle.0).input_cost;
    let _pending = reconcile_budget_reservation(
        counters,
        handle,
        input_cost,
        ReservationFinality::Finalize,
        AdjustmentMode::Apply,
    )
    .await?;
    Ok(())
}

// invalidate_budget_reservation_counters: a release that failed leaves the reserved counters dropped directly
pub async fn invalidate_reservation_keys<C>(counters: &C, handle: &ReservationHandle)
where
    C: SettlementCounters,
{
    for counter_key in get_reserved_counter_keys(handle) {
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
    if lock(&handle.0).callback_bound {
        return;
    }
    release_or_invalidate(counters, handle).await;
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
