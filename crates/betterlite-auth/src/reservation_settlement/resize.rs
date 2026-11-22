use rust_decimal::Decimal;

use super::adjust::{
    AdjustmentMode, InconsistentCounterPolicy, SettlementOptions, set_entries_actual_cost,
};
use super::counters::SettlementCounters;
use super::entry::ReservationEntry;
use super::error::SettlementError;

// _resize_applied_reservation: a resize against an inconsistent counter refuses, since the DB floor lacks the in-flight cost
pub async fn resize_applied_reservation<C>(
    counters: &C,
    entries: &mut [ReservationEntry],
    current_reserved_cost: Decimal,
    new_reserved_cost: Decimal,
) -> Result<(), SettlementError>
where
    C: SettlementCounters,
{
    let options = SettlementOptions {
        inconsistent: InconsistentCounterPolicy::Reject,
        mode: AdjustmentMode::Apply,
    };
    set_entries_actual_cost(
        counters,
        entries,
        new_reserved_cost,
        current_reserved_cost,
        options,
    )
    .await?;
    for entry in entries {
        entry.reserved_cost = Some(new_reserved_cost);
        entry.applied_adjustment = Decimal::ZERO;
    }
    Ok(())
}
