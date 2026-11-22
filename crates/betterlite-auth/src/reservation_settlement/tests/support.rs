use std::collections::{BTreeMap, BTreeSet};
use std::mem;
use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_redis::request_batch::lock;
use rust_decimal::Decimal;

use crate::budget_counter::CounterEntityType;
use crate::reservation_settlement::adjust::{
    AdjustmentMode, InconsistentCounterPolicy, SettlementOptions,
};
use crate::reservation_settlement::counters::{ScopeHold, SettlementCounters};
use crate::reservation_settlement::entry::ReservationEntry;
use crate::reservation_settlement::error::{CounterReadError, SettlementError};
use crate::reservation_settlement::handle::ReservationHandle;
use crate::spend_counter_batch::batch::PendingSpendIncrement;
use crate::spend_writes::error::{LayersError, SpendWriteError};

pub(crate) const K1: &str = "spend:key:k1";
pub(crate) const K2: &str = "spend:key:k2";
pub(crate) const APPLY: SettlementOptions = SettlementOptions {
    inconsistent: InconsistentCounterPolicy::Reseed,
    mode: AdjustmentMode::Apply,
};
pub(crate) const DEFER: SettlementOptions = SettlementOptions {
    inconsistent: InconsistentCounterPolicy::Reseed,
    mode: AdjustmentMode::Defer,
};
pub(crate) const REJECT: SettlementOptions = SettlementOptions {
    inconsistent: InconsistentCounterPolicy::Reject,
    mode: AdjustmentMode::Apply,
};

pub(crate) fn decimal(text: &str) -> Decimal {
    text.parse().expect("a fixture decimal")
}

pub(crate) fn entry(counter_key: &str, reserved_cost: &str) -> ReservationEntry {
    ReservationEntry {
        counter_key: Some(counter_key.to_owned()),
        entity_type: CounterEntityType::Key,
        entity_id: "key-1".to_owned(),
        reserved_cost: Some(decimal(reserved_cost)),
        applied_adjustment: Decimal::ZERO,
    }
}

pub(crate) fn handle(
    reserved_cost: &str,
    entries: Vec<ReservationEntry>,
    input_cost: &str,
) -> ReservationHandle {
    ReservationHandle::new(decimal(reserved_cost), entries, decimal(input_cost))
}

pub(crate) fn reserved(counter_key: &str, amount: &str) -> (FakeCounters, ReservationHandle) {
    let fake = counters(&[(counter_key, amount)]);
    let reservation = handle(amount, vec![entry(counter_key, amount)], "3");
    (fake, reservation)
}

pub(crate) fn is_finalized(handle: &ReservationHandle) -> bool {
    lock(&handle.0).finalized
}

pub(crate) fn entries_of(handle: &ReservationHandle) -> Vec<ReservationEntry> {
    lock(&handle.0).entries.clone()
}

#[derive(Clone, Default)]
pub(crate) struct Calls {
    pub(crate) scopes: Vec<BTreeSet<String>>,
    pub(crate) reads: Vec<String>,
    pub(crate) pipelines: Vec<Vec<(String, Decimal)>>,
    pub(crate) increments: Vec<(String, Decimal)>,
    pub(crate) invalidations: Vec<String>,
    pub(crate) reseeds: Vec<String>,
}

pub(crate) struct FakeCounters {
    pub(crate) values: Mutex<BTreeMap<String, Decimal>>,
    pub(crate) unreadable: BTreeSet<String>,
    pub(crate) reseeded: BTreeSet<String>,
    pub(crate) pipeline_failures: BTreeSet<String>,
    pub(crate) calls: Mutex<Calls>,
}

pub(crate) fn counters(values: &[(&str, &str)]) -> FakeCounters {
    let mut held = BTreeMap::new();
    for (counter_key, amount) in values {
        held.insert((*counter_key).to_owned(), decimal(amount));
    }
    FakeCounters {
        values: Mutex::new(held),
        unreadable: BTreeSet::new(),
        reseeded: BTreeSet::new(),
        pipeline_failures: BTreeSet::new(),
        calls: Mutex::new(Calls::default()),
    }
}

pub(crate) fn take_calls(counters: &FakeCounters) -> Calls {
    let mut held = lock(&counters.calls);
    mem::take(&mut *held)
}

pub(crate) fn value_of(counters: &FakeCounters, counter_key: &str) -> Option<Decimal> {
    lock(&counters.values).get(counter_key).copied()
}

fn store_failure() -> SettlementError {
    SettlementError::Write(SpendWriteError::from(LayersError::Unavailable))
}

pub(crate) struct FakeScope;

impl ScopeHold for FakeScope {}

#[async_trait]
impl SettlementCounters for FakeCounters {
    fn enter_counters_batch_scope(
        &self,
        counter_keys: &BTreeSet<String>,
    ) -> Box<dyn ScopeHold + Send + '_> {
        lock(&self.calls).scopes.push(counter_keys.clone());
        Box::new(FakeScope)
    }

    async fn read_cache_value(
        &self,
        counter_key: &str,
    ) -> Result<Option<Decimal>, CounterReadError> {
        lock(&self.calls).reads.push(counter_key.to_owned());
        if self.unreadable.contains(counter_key) {
            return Err(CounterReadError::NotNumeric {
                counter_key: counter_key.to_owned(),
            });
        }
        Ok(lock(&self.values).get(counter_key).copied())
    }

    async fn reseed_from_db(&self, counter_key: &str) -> bool {
        lock(&self.calls).reseeds.push(counter_key.to_owned());
        self.reseeded.contains(counter_key)
    }

    async fn increment_counter(
        &self,
        counter_key: &str,
        increment: Decimal,
    ) -> Result<(), SettlementError> {
        lock(&self.calls)
            .increments
            .push((counter_key.to_owned(), increment));
        let held = lock(&self.values).get(counter_key).copied();
        let current = held.unwrap_or(Decimal::ZERO);
        let after = current
            .checked_add(increment)
            .ok_or(SettlementError::Overflow)?;
        let _previous = lock(&self.values).insert(counter_key.to_owned(), after);
        Ok(())
    }

    async fn increment_pipeline(
        &self,
        pending: &[PendingSpendIncrement],
    ) -> Result<(), SettlementError> {
        let mut snapshot = Vec::new();
        for item in pending {
            snapshot.push((item.counter_key.clone(), item.increment));
        }
        lock(&self.calls).pipelines.push(snapshot);
        for item in pending {
            if self.pipeline_failures.contains(&item.counter_key) {
                return Err(store_failure());
            }
        }
        for item in pending {
            let held = lock(&self.values).get(&item.counter_key).copied();
            let current = held.unwrap_or(Decimal::ZERO);
            let after = current
                .checked_add(item.increment)
                .ok_or(SettlementError::Overflow)?;
            let _previous = lock(&self.values).insert(item.counter_key.clone(), after);
        }
        Ok(())
    }

    async fn invalidate(&self, counter_key: &str) {
        lock(&self.calls).invalidations.push(counter_key.to_owned());
        let _removed = lock(&self.values).remove(counter_key);
    }
}
