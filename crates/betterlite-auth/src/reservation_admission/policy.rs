use std::slice;

use betterlite_error::error::details::BudgetExceededDetails;
use rust_decimal::Decimal;
use secrecy::ExposeSecret as _;

use crate::budget_counter::{BudgetCounter, CounterEntityType};
use crate::budget_primitives::throttle::should_throttle;
use crate::reservation_settlement::entry::ReservationEntry;
use crate::reservation_settlement::release::release_entries_best_effort;
use crate::reservation_settlement::resize::resize_applied_reservation;

use super::counters::AdmissionCounters;
use super::engine::AdmissionRequest;
use super::error::AdmissionError;

// LiteLLM's 1e-12: a float sum may miss the budget by rounding, and a difference that small still counts as inside it
const TOLERANCE: Decimal = Decimal::from_parts(1, 0, 0, false, 12);

// one counter the reservation drove over its budget, with the entry it holds and the cost the reservation still carries
pub(crate) struct OverBudget<'a> {
    pub(crate) counter: &'a BudgetCounter,
    pub(crate) entry: ReservationEntry,
    pub(crate) reservation_cost: Decimal,
    pub(crate) current_spend: Decimal,
}

// the counter's spend before this reservation and the budget it leaves
struct Headroom {
    spent_before: Decimal,
    remaining: Decimal,
}

// _apply_over_budget_reservation_policy: a throttled key releases its own reservation, and every other over-budget counter blocks when nothing is left, rejects an estimate strict enforcement cannot fit, or resizes to the headroom
pub(crate) async fn apply_over_budget<C>(
    counters: &C,
    request: &AdmissionRequest<'_>,
    over: &OverBudget<'_>,
    applied_entries: &mut Vec<ReservationEntry>,
) -> Result<Decimal, AdmissionError>
where
    C: AdmissionCounters,
{
    if release_for_throttle(request, over.counter) {
        release_throttled(counters, over, applied_entries).await;
        return Ok(over.reservation_cost);
    }
    let headroom = headroom(over)?;
    if headroom.remaining <= TOLERANCE {
        return Err(budget_exceeded(
            request,
            over.counter,
            over.current_spend,
            None,
        ));
    }
    if strict_rejection(request, over)? {
        return Err(budget_exceeded(
            request,
            over.counter,
            headroom.spent_before,
            Some(over.reservation_cost),
        ));
    }
    resize_applied_reservation(
        counters,
        applied_entries,
        over.reservation_cost,
        headroom.remaining,
    )
    .await?;
    Ok(headroom.remaining)
}

// the key's own reservation goes back to the rate limiter, and the entry leaves the applied list
async fn release_throttled<C>(
    counters: &C,
    over: &OverBudget<'_>,
    applied_entries: &mut Vec<ReservationEntry>,
) where
    C: AdmissionCounters,
{
    let mut entry = over.entry.clone();
    release_entries_best_effort(counters, slice::from_mut(&mut entry), over.reservation_cost).await;
    remove_entry(applied_entries, &over.counter.counter_key);
}

// strict enforcement refuses an estimate that overshoots the budget beyond the tolerance
fn strict_rejection(
    request: &AdmissionRequest<'_>,
    over: &OverBudget<'_>,
) -> Result<bool, AdmissionError> {
    let overshoot = over
        .current_spend
        .checked_sub(over.counter.max_budget)
        .ok_or(AdmissionError::Overflow)?;
    Ok(request.fail_closed_budget_enforcement && overshoot > TOLERANCE)
}

// _key_reservation_should_release_for_throttle: only the key's own max_budget counter makes way for the rate limiter
fn release_for_throttle(request: &AdmissionRequest<'_>, counter: &BudgetCounter) -> bool {
    let Some(token) = request.token else {
        return false;
    };
    let Some(secret) = token.view.token.token.as_ref() else {
        return false;
    };
    let own_counter = format!("spend:key:{}", secret.expose_secret());
    counter.counter_key == own_counter && should_throttle(token, request.throttle_percentage)
}

fn headroom(over: &OverBudget<'_>) -> Result<Headroom, AdmissionError> {
    let spent_before = over
        .current_spend
        .checked_sub(over.reservation_cost)
        .ok_or(AdmissionError::Overflow)?;
    let remaining = over
        .counter
        .max_budget
        .checked_sub(spent_before)
        .ok_or(AdmissionError::Overflow)?;
    Ok(Headroom {
        spent_before,
        remaining,
    })
}

// _raise_counter_budget_exceeded: the rejection LiteLLM raises with the entity and the cost it read
fn budget_exceeded(
    request: &AdmissionRequest<'_>,
    counter: &BudgetCounter,
    current_cost: Decimal,
    estimated_cost: Option<Decimal>,
) -> AdmissionError {
    let estimate = estimated_cost.map_or(String::new(), |cost| {
        format!("Estimated request cost: {cost}, ")
    });
    let entity_type = litellm_entity_name(&counter.entity_type);
    let message = format!(
        "Budget has been exceeded! {entity_type}={} Current cost: {current_cost}, {estimate}Max budget: {}",
        counter.entity_id, counter.max_budget
    );
    AdmissionError::BudgetExceeded {
        details: Box::new(BudgetExceededDetails {
            current_cost,
            max_budget: counter.max_budget,
            message: Some(message),
            provider: String::new(),
            entity_type: Some(counter.entity_type.wire_value().to_owned()),
            entity_id: Some(rejected_entity_id(counter)),
            status_code: request.budget_exceeded_status,
        }),
    }
}

// spend_log_entity_id or entity_id: a window counter names the entity whose window it holds
fn rejected_entity_id(counter: &BudgetCounter) -> String {
    counter
        .spend_log_entity_id
        .clone()
        .unwrap_or_else(|| counter.entity_id.clone())
}

// the name a counter's entity type carries in LiteLLM's message, the reverse of _COUNTER_ENTITY_TYPES
const fn litellm_entity_name(entity_type: &CounterEntityType) -> &'static str {
    match entity_type {
        CounterEntityType::Key => "Key",
        CounterEntityType::Team => "Team",
        CounterEntityType::TeamMember => "TeamMember",
        CounterEntityType::User => "User",
        CounterEntityType::EndUser => "EndUser",
        CounterEntityType::Tag => "Tag",
        CounterEntityType::ModelAccessGroup => "Model access group",
        CounterEntityType::Organization => "Organization",
        CounterEntityType::Project => "Project",
    }
}

fn remove_entry(applied_entries: &mut Vec<ReservationEntry>, counter_key: &str) {
    applied_entries.retain(|entry| entry.counter_key.as_deref() != Some(counter_key));
}
