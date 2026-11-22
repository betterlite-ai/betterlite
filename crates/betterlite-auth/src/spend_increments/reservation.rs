use std::collections::BTreeSet;

use betterlite_redis::request_batch::lock;
use rust_decimal::Decimal;

use crate::reservation_settlement::adjust::AdjustmentMode;
use crate::reservation_settlement::counters::SettlementCounters;
use crate::reservation_settlement::handle::{ReservationHandle, get_reserved_counter_keys};
use crate::reservation_settlement::reconcile::{ReservationFinality, reconcile_budget_reservation};
use crate::reservation_settlement::release::invalidate_reservation_keys;
use crate::spend_counter_batch::batch::PendingSpendIncrement;

// _ReservationCounterUpdate: the reserved counters the direct increment must skip, and the adjustments that settle them, still to be written
#[derive(Default)]
pub(super) struct ReservationCounterUpdate {
    pub(super) reserved_counter_keys: BTreeSet<String>,
    pub(super) pending: Vec<PendingSpendIncrement>,
}

// _reconcile_budget_reservation_for_counter_update: a failed reconcile drops the reserved counters so the direct increments still charge them
pub(super) async fn reconcile_reservation<Counters>(
    counters: &Counters,
    handle: Option<&ReservationHandle>,
    response_cost: Option<Decimal>,
) -> ReservationCounterUpdate
where
    Counters: SettlementCounters,
{
    let Some(handle) = handle else {
        return ReservationCounterUpdate::default();
    };
    let finalized = lock(&handle.0).finalized;
    if finalized {
        return ReservationCounterUpdate::default();
    }
    let reserved_counter_keys = get_reserved_counter_keys(handle);
    let actual_cost = response_cost.unwrap_or(Decimal::ZERO);
    match reconcile_budget_reservation(
        counters,
        handle,
        actual_cost,
        ReservationFinality::LeaveOpen,
        AdjustmentMode::Defer,
    )
    .await
    {
        Ok(pending) => ReservationCounterUpdate {
            reserved_counter_keys,
            pending,
        },
        Err(error) => {
            tracing::warn!(
                %error,
                "Failed to reconcile budget reservation after persisted spend; invalidating reserved counters and falling back to direct increment"
            );
            invalidate_reservation_keys(counters, handle).await;
            ReservationCounterUpdate::default()
        }
    }
}
