use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use betterlite_redis::request_batch::lock;
use rust_decimal::Decimal;

use super::entry::{ReservationEntry, get_entry_reserved_cost};
use super::error::SettlementError;

// the reservation dict every settlement path shares, one dict per request like LiteLLM's
#[derive(Clone, Debug)]
pub struct ReservationHandle(pub Arc<Mutex<ReservationState>>);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReservationState {
    pub reserved_cost: Decimal,
    pub entries: Vec<ReservationEntry>,
    pub finalized: bool,
    pub callback_bound: bool,
    pub input_cost: Decimal,
}

impl ReservationHandle {
    #[must_use]
    pub fn new(
        reserved_cost: Decimal,
        entries: Vec<ReservationEntry>,
        input_cost: Decimal,
    ) -> Self {
        Self(Arc::new(Mutex::new(ReservationState {
            reserved_cost,
            entries,
            finalized: false,
            callback_bound: false,
            input_cost,
        })))
    }
}

// get_reserved_counter_keys: every counter a parallel settlement pass may touch, each once
#[must_use]
pub fn get_reserved_counter_keys(handle: &ReservationHandle) -> BTreeSet<String> {
    let state = lock(&handle.0);
    let mut counter_keys = BTreeSet::new();
    for entry in &state.entries {
        if let Some(counter_key) = &entry.counter_key {
            counter_keys.insert(counter_key.clone());
        }
    }
    drop(state);
    counter_keys
}

// stamp_budget_reservation_actual_cost: the deferred adjustments are already written, so only the record moves
pub fn stamp_reservation_actual_cost(
    handle: &ReservationHandle,
    actual_cost: Decimal,
) -> Result<(), SettlementError> {
    let mut state = lock(&handle.0);
    let default_reserved_cost = state.reserved_cost;
    for entry in &mut state.entries {
        let reserved_cost = get_entry_reserved_cost(entry, default_reserved_cost);
        let target = actual_cost
            .checked_sub(reserved_cost)
            .ok_or(SettlementError::Overflow)?;
        entry.applied_adjustment = target;
    }
    drop(state);
    Ok(())
}
