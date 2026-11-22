use std::collections::BTreeSet;

use rust_decimal::Decimal;

use crate::spend_counter_batch::batch::PendingSpendIncrement;

use super::counters::SettlementCounters;
use super::entry::{ReservationEntry, get_entry_reserved_cost};
use super::error::SettlementError;

// LiteLLM's -1e-12: a float sum may undershoot zero by rounding, and the counter still absorbs the delta
const NEGATIVE_ADJUSTMENT_FLOOR: Decimal = Decimal::from_parts(1, 0, 0, true, 12);

// _EntryAdjustment, frozen like LiteLLM's dataclass
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EntryAdjustment {
    pub entry_index: usize,
    pub counter_key: String,
    pub target_adjustment: Decimal,
    pub adjustment: Decimal,
}

// _set_reserved_entries_actual_cost's reseed_on_inconsistent: restore the DB floor, or refuse the settle
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InconsistentCounterPolicy {
    Reseed,
    Reject,
}

// _set_reserved_entries_actual_cost's apply_consistent: write the consistent deltas here, or defer them to the caller
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdjustmentMode {
    Apply,
    Defer,
}

// the two switches of _set_reserved_entries_actual_cost, one struct since clippy allows a function only one bool
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SettlementOptions {
    pub inconsistent: InconsistentCounterPolicy,
    pub mode: AdjustmentMode,
}

// _entry_adjustment: an entry with no counter key or no delta contributes nothing to this settle
pub fn entry_adjustment(
    entry: &ReservationEntry,
    entry_index: usize,
    actual_cost: Decimal,
    default_reserved_cost: Decimal,
) -> Result<Option<EntryAdjustment>, SettlementError> {
    let Some(counter_key) = entry.counter_key.as_deref() else {
        return Ok(None);
    };
    let target_adjustment = actual_cost
        .checked_sub(get_entry_reserved_cost(entry, default_reserved_cost))
        .ok_or(SettlementError::Overflow)?;
    let adjustment = target_adjustment
        .checked_sub(entry.applied_adjustment)
        .ok_or(SettlementError::Overflow)?;
    if adjustment.is_zero() {
        return Ok(None);
    }
    Ok(Some(EntryAdjustment {
        entry_index,
        counter_key: counter_key.to_owned(),
        target_adjustment,
        adjustment,
    }))
}

// _counter_can_apply_adjustment: a read failure, a missing value or a delta below zero settles by reseed instead
pub async fn counter_can_apply_adjustment<C>(
    counters: &C,
    counter_key: &str,
    adjustment: Decimal,
) -> bool
where
    C: SettlementCounters,
{
    let Ok(Some(current_value)) = counters.read_cache_value(counter_key).await else {
        return false;
    };
    if adjustment >= Decimal::ZERO {
        return true;
    }
    let Some(after) = current_value.checked_add(adjustment) else {
        return false;
    };
    after >= NEGATIVE_ADJUSTMENT_FLOOR
}

// _reseed_reserved_entry: the counter was flushed or reseeded, so the DB floor plus the cost already billed is the truth
pub async fn reseed_reserved_entry<C>(
    counters: &C,
    item: &EntryAdjustment,
    actual_cost: Decimal,
) -> Result<(), SettlementError>
where
    C: SettlementCounters,
{
    let reseeded = counters.reseed_from_db(&item.counter_key).await;
    if reseeded && actual_cost > Decimal::ZERO {
        counters
            .increment_counter(&item.counter_key, actual_cost)
            .await?;
    }
    Ok(())
}

// _set_reserved_entries_actual_cost: one batch scope over every counter, consistent deltas in one pipeline, the rest reseeded
pub async fn set_entries_actual_cost<C>(
    counters: &C,
    entries: &mut [ReservationEntry],
    actual_cost: Decimal,
    default_reserved_cost: Decimal,
    options: SettlementOptions,
) -> Result<Vec<PendingSpendIncrement>, SettlementError>
where
    C: SettlementCounters,
{
    let counter_keys: BTreeSet<String> = entries
        .iter()
        .filter_map(|entry| entry.counter_key.clone())
        .collect();
    let _hold = counters.enter_counters_batch_scope(&counter_keys);
    let adjustments = collect_adjustments(entries, actual_cost, default_reserved_cost)?;
    let applicable = applicable_flags(counters, &adjustments, options.inconsistent).await?;
    let pending = write_applicable(counters, &adjustments, &applicable, options.mode).await?;
    for (item, can_apply) in adjustments.iter().zip(&applicable) {
        if !can_apply {
            reseed_reserved_entry(counters, item, actual_cost).await?;
        }
    }
    for (item, can_apply) in adjustments.iter().zip(&applicable) {
        if options.mode == AdjustmentMode::Defer && *can_apply {
            continue;
        }
        let Some(entry) = entries.get_mut(item.entry_index) else {
            continue;
        };
        entry.applied_adjustment = item.target_adjustment;
    }
    if options.mode == AdjustmentMode::Apply {
        return Ok(Vec::new());
    }
    Ok(pending)
}

// _counter_can_apply_adjustment for every adjustment: one read each, and a Reject policy names the first inconsistent counter
async fn applicable_flags<C>(
    counters: &C,
    adjustments: &[EntryAdjustment],
    inconsistent: InconsistentCounterPolicy,
) -> Result<Vec<bool>, SettlementError>
where
    C: SettlementCounters,
{
    let mut applicable = Vec::with_capacity(adjustments.len());
    for item in adjustments {
        let can_apply =
            counter_can_apply_adjustment(counters, &item.counter_key, item.adjustment).await;
        applicable.push(can_apply);
    }
    if inconsistent == InconsistentCounterPolicy::Reject {
        for (item, can_apply) in adjustments.iter().zip(&applicable) {
            if !can_apply {
                return Err(SettlementError::InconsistentCounter {
                    counter_key: item.counter_key.clone(),
                });
            }
        }
    }
    Ok(applicable)
}

fn collect_adjustments(
    entries: &[ReservationEntry],
    actual_cost: Decimal,
    default_reserved_cost: Decimal,
) -> Result<Vec<EntryAdjustment>, SettlementError> {
    let mut adjustments = Vec::new();
    for (entry_index, entry) in entries.iter().enumerate() {
        if let Some(item) =
            entry_adjustment(entry, entry_index, actual_cost, default_reserved_cost)?
        {
            adjustments.push(item);
        }
    }
    Ok(adjustments)
}

// the consistent deltas, written in one pipeline only when the caller applies them
async fn write_applicable<C>(
    counters: &C,
    adjustments: &[EntryAdjustment],
    applicable: &[bool],
    mode: AdjustmentMode,
) -> Result<Vec<PendingSpendIncrement>, SettlementError>
where
    C: SettlementCounters,
{
    let mut pending = Vec::new();
    for (item, can_apply) in adjustments.iter().zip(applicable) {
        if *can_apply {
            pending.push(PendingSpendIncrement {
                counter_key: item.counter_key.clone(),
                increment: item.adjustment,
            });
        }
    }
    if mode == AdjustmentMode::Apply && !pending.is_empty() {
        counters.increment_pipeline(&pending).await?;
    }
    Ok(pending)
}
