use betterlite_redis::request_batch::lock;
use rust_decimal::Decimal;

use crate::spend_counter_batch::batch::PendingSpendIncrement;

use super::adjust::{
    AdjustmentMode, InconsistentCounterPolicy, SettlementOptions, set_entries_actual_cost,
};
use super::counters::SettlementCounters;
use super::entry::ReservationEntry;
use super::error::SettlementError;
use super::handle::ReservationHandle;

// reconcile_budget_reservation's finalize: a release settles for good, a resize keeps the record open for the real cost
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReservationFinality {
    Finalize,
    LeaveOpen,
}

// the terminal settle both release paths take: an already finalized reservation stays untouched
pub async fn settle_reservation_to<C>(
    counters: &C,
    handle: &ReservationHandle,
    actual_cost: Decimal,
) -> Result<(), SettlementError>
where
    C: SettlementCounters,
{
    if lock(&handle.0).finalized {
        return Ok(());
    }
    let _pending = reconcile_budget_reservation(
        counters,
        handle,
        actual_cost,
        ReservationFinality::Finalize,
        AdjustmentMode::Apply,
    )
    .await?;
    Ok(())
}

// reconcile_budget_reservation: settle every reserved entry on the actual cost, then stamp the record
pub async fn reconcile_budget_reservation<C>(
    counters: &C,
    handle: &ReservationHandle,
    actual_cost: Decimal,
    finality: ReservationFinality,
    mode: AdjustmentMode,
) -> Result<Vec<PendingSpendIncrement>, SettlementError>
where
    C: SettlementCounters,
{
    let Some((reserved_cost, mut entries)) = claim_settlement(handle) else {
        return Ok(Vec::new());
    };
    let options = SettlementOptions {
        inconsistent: InconsistentCounterPolicy::Reseed,
        mode,
    };
    let outcome =
        set_entries_actual_cost(counters, &mut entries, actual_cost, reserved_cost, options).await;
    {
        let mut state = lock(&handle.0);
        if outcome.is_ok() {
            state.entries = entries;
            if finality == ReservationFinality::Finalize {
                state.finalized = true;
            }
        }
    }
    outcome
}

// the finalized guard makes a settled reservation a no-op, so a second terminal path cannot apply the deltas twice
fn claim_settlement(handle: &ReservationHandle) -> Option<(Decimal, Vec<ReservationEntry>)> {
    let state = lock(&handle.0);
    if state.finalized {
        return None;
    }
    Some((state.reserved_cost, state.entries.clone()))
}
