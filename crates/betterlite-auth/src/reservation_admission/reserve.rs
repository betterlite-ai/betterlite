use std::collections::BTreeMap;

use rust_decimal::Decimal;

use crate::budget_counter::BudgetCounter;
use crate::reservation_settlement::entry::{ReservationEntry, counter_to_reservation_entry};
use crate::spend_counter_batch::batch::PendingSpendIncrement;

use super::counters::AdmissionCounters;
use super::engine::AdmissionRequest;
use super::error::{AdmissionError, unavailable};
use super::groups::reservation_groups;
use super::policy::{OverBudget, apply_over_budget};

// one group's counters with the cost it charges and what the reservation pipeline reported for each of them
pub(crate) struct GroupCharge<'a> {
    pub(crate) counters: &'a [&'a BudgetCounter],
    pub(crate) current_spend: &'a BTreeMap<String, Decimal>,
    pub(crate) cost: Decimal,
    pub(crate) reserved_values: Vec<Option<Decimal>>,
}

// _reserve_reservable_counters: read every counter once, then charge the groups in order so each group's policy settles before the next is charged
pub(crate) async fn reserve_reservable_counters<C>(
    counters: &C,
    request: &AdmissionRequest<'_>,
    reservable: &[&BudgetCounter],
    applied_entries: &mut Vec<ReservationEntry>,
) -> Result<Decimal, AdmissionError>
where
    C: AdmissionCounters,
{
    let current_spend = read_current_spend(counters, reservable).await;
    let groups = reservation_groups(reservable, &current_spend, request.reservation_cost);
    charge_groups(counters, request, &groups, &current_spend, applied_entries).await
}

// every group is charged in order, each one starting from the cost the one before it left standing
async fn charge_groups<C>(
    counters: &C,
    request: &AdmissionRequest<'_>,
    groups: &[Vec<&BudgetCounter>],
    current_spend: &BTreeMap<String, Decimal>,
    applied_entries: &mut Vec<ReservationEntry>,
) -> Result<Decimal, AdmissionError>
where
    C: AdmissionCounters,
{
    let mut reservation_cost = request.reservation_cost;
    for group in groups {
        let mut charge = GroupCharge {
            counters: group,
            current_spend,
            cost: reservation_cost,
            reserved_values: Vec::new(),
        };
        reservation_cost = charge_group(counters, request, &mut charge, applied_entries).await?;
    }
    Ok(reservation_cost)
}

// _get_current_counter_value for every reservable counter, before any of them is charged
async fn read_current_spend<C>(
    counters: &C,
    reservable: &[&BudgetCounter],
) -> BTreeMap<String, Decimal>
where
    C: AdmissionCounters,
{
    let mut current_spend = BTreeMap::new();
    for counter in reservable {
        let spend = counters
            .current_spend(&counter.counter_key, counter.fallback_spend)
            .await;
        current_spend.insert(counter.counter_key.clone(), spend);
    }
    current_spend
}

// one pipeline reserves the whole group; a failed one drops the group's entries and, under strict enforcement, rejects the request
async fn charge_group<C>(
    counters: &C,
    request: &AdmissionRequest<'_>,
    charge: &mut GroupCharge<'_>,
    applied_entries: &mut Vec<ReservationEntry>,
) -> Result<Decimal, AdmissionError>
where
    C: AdmissionCounters,
{
    let cost = charge.cost;
    applied_entries.extend(
        charge
            .counters
            .iter()
            .map(|counter| counter_to_reservation_entry(counter, cost)),
    );
    let outcome = counters
        .reserve_pipeline(&pipeline_increments(charge.counters, cost))
        .await;
    let values = match outcome {
        Ok(values) => values,
        Err(error) => {
            tracing::warn!(
                %error,
                "Skipping budget reservation because spend counter reservation failed"
            );
            invalidate_group(counters, charge.counters).await;
            remove_group_entries(applied_entries, charge.counters);
            return failed_group(request, charge.counters, cost);
        }
    };
    charge.reserved_values = values;
    settle_group(counters, request, charge, applied_entries).await
}

// the over-budget policy runs for every counter of the group whose reservation landed over its budget
pub(crate) async fn settle_group<C>(
    counters: &C,
    request: &AdmissionRequest<'_>,
    charge: &GroupCharge<'_>,
    applied_entries: &mut Vec<ReservationEntry>,
) -> Result<Decimal, AdmissionError>
where
    C: AdmissionCounters,
{
    let mut reservation_cost = charge.cost;
    for (index, counter) in charge.counters.iter().enumerate() {
        if let Some(over) = over_budget(charge, counter, index, reservation_cost, applied_entries)?
        {
            reservation_cost = apply_over_budget(counters, request, &over, applied_entries).await?;
        }
    }
    Ok(reservation_cost)
}

// the policy's input when the counter's reservation landed over its budget, and nothing for a counter the group lost
fn over_budget<'a>(
    charge: &'a GroupCharge<'_>,
    counter: &'a BudgetCounter,
    index: usize,
    reservation_cost: Decimal,
    applied_entries: &[ReservationEntry],
) -> Result<Option<OverBudget<'a>>, AdmissionError> {
    let Some(entry) = find_entry(applied_entries, &counter.counter_key) else {
        return Ok(None);
    };
    let reserved_value = charge.reserved_values.get(index).copied().flatten();
    let Some(current_spend) = reserved_spend(counter, charge, reservation_cost, reserved_value)?
    else {
        return Ok(None);
    };
    if current_spend <= counter.max_budget {
        return Ok(None);
    }
    Ok(Some(OverBudget {
        counter,
        entry,
        reservation_cost,
        current_spend,
    }))
}

// reserved_value - (charged_cost - reservation_cost), or the pre-read plus the standing cost when the pipeline answered nothing
fn reserved_spend(
    counter: &BudgetCounter,
    charge: &GroupCharge<'_>,
    reservation_cost: Decimal,
    reserved_value: Option<Decimal>,
) -> Result<Option<Decimal>, AdmissionError> {
    let Some(value) = reserved_value else {
        let Some(pre_reservation) = charge.current_spend.get(&counter.counter_key) else {
            return Ok(None);
        };
        let current = pre_reservation
            .checked_add(reservation_cost)
            .ok_or(AdmissionError::Overflow)?;
        return Ok(Some(current));
    };
    let since_charge = charge
        .cost
        .checked_sub(reservation_cost)
        .ok_or(AdmissionError::Overflow)?;
    let current = value
        .checked_sub(since_charge)
        .ok_or(AdmissionError::Overflow)?;
    Ok(Some(current))
}

// a failed pipeline rejects under strict enforcement, else the next group is charged as if this one never existed
fn failed_group(
    request: &AdmissionRequest<'_>,
    group: &[&BudgetCounter],
    cost: Decimal,
) -> Result<Decimal, AdmissionError> {
    let Some(first) = group.first() else {
        return Ok(cost);
    };
    if request.fail_closed_budget_enforcement {
        return Err(unavailable(&first.counter_key));
    }
    Ok(cost)
}

fn pipeline_increments(group: &[&BudgetCounter], cost: Decimal) -> Vec<PendingSpendIncrement> {
    group
        .iter()
        .map(|counter| PendingSpendIncrement {
            counter_key: counter.counter_key.clone(),
            increment: cost,
        })
        .collect()
}

async fn invalidate_group<C>(counters: &C, group: &[&BudgetCounter])
where
    C: AdmissionCounters,
{
    for counter in group {
        counters.invalidate(&counter.counter_key).await;
    }
}

fn remove_group_entries(applied_entries: &mut Vec<ReservationEntry>, group: &[&BudgetCounter]) {
    applied_entries.retain(|entry| {
        let Some(counter_key) = entry.counter_key.as_deref() else {
            return true;
        };
        !group
            .iter()
            .any(|counter| counter.counter_key == counter_key)
    });
}

fn find_entry(applied_entries: &[ReservationEntry], counter_key: &str) -> Option<ReservationEntry> {
    applied_entries
        .iter()
        .find(|entry| entry.counter_key.as_deref() == Some(counter_key))
        .cloned()
}
