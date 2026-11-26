#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_logging::reservation_binding::ReservationBinder as _;
use betterlite_models::decisions::Json;
use betterlite_redis::request_batch::lock;
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;

use crate::budget_counter::CounterEntityType;
use crate::reservation_settlement::counters::{ScopeHold, SettlementCounters};
use crate::reservation_settlement::entry::ReservationEntry;
use crate::reservation_settlement::error::{CounterReadError, SettlementError};
use crate::reservation_settlement::handle::ReservationHandle;
use crate::reservation_settlement::release::release_unbound_reservation;
use crate::spend_counter_batch::batch::PendingSpendIncrement;

use super::RequestReservationBinder;

const COUNTER: &str = "spend:key:k1";
const BOTH_BUCKETS: &str = r#"{"metadata": {"user_api_key_budget_reservation": {"id": "r1"}}, "litellm_metadata": {"user_api_key_auth": {"budget_reservation": {"id": "r1"}}}}"#;
const BOTH_BOUND: &str = r#"{"metadata": {"user_api_key_budget_reservation": {"id": "r1", "callback_bound": true}}, "litellm_metadata": {"user_api_key_auth": {"budget_reservation": {"id": "r1", "callback_bound": true}}}}"#;

struct NoScope;

impl ScopeHold for NoScope {}

// every counter call a settlement makes, so a bound reservation can be shown to make none
struct RecordingCounters {
    values: Mutex<BTreeMap<String, Decimal>>,
    calls: Mutex<Vec<String>>,
}

impl RecordingCounters {
    fn holding(amount: Decimal) -> Self {
        Self {
            values: Mutex::new(BTreeMap::from([(COUNTER.to_owned(), amount)])),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn record(&self, call: String) {
        lock(&self.calls).push(call);
    }

    fn add(&self, counter_key: &str, increment: Decimal) -> Result<(), SettlementError> {
        let mut values = lock(&self.values);
        let current = values.get(counter_key).copied().unwrap_or(Decimal::ZERO);
        let after = current
            .checked_add(increment)
            .ok_or(SettlementError::Overflow)?;
        let _previous = values.insert(counter_key.to_owned(), after);
        drop(values);
        Ok(())
    }
}

#[async_trait]
impl SettlementCounters for RecordingCounters {
    fn enter_counters_batch_scope(
        &self,
        counter_keys: &BTreeSet<String>,
    ) -> Box<dyn ScopeHold + Send + '_> {
        self.record(format!("scope {counter_keys:?}"));
        Box::new(NoScope)
    }

    async fn read_cache_value(
        &self,
        counter_key: &str,
    ) -> Result<Option<Decimal>, CounterReadError> {
        self.record(format!("read {counter_key}"));
        Ok(lock(&self.values).get(counter_key).copied())
    }

    async fn reseed_from_db(&self, counter_key: &str) -> bool {
        self.record(format!("reseed {counter_key}"));
        false
    }

    async fn increment_counter(
        &self,
        counter_key: &str,
        increment: Decimal,
    ) -> Result<(), SettlementError> {
        self.record(format!("increment {counter_key} {increment}"));
        self.add(counter_key, increment)
    }

    async fn increment_pipeline(
        &self,
        pending: &[PendingSpendIncrement],
    ) -> Result<(), SettlementError> {
        for item in pending {
            self.record(format!("pipeline {} {}", item.counter_key, item.increment));
            self.add(&item.counter_key, item.increment)?;
        }
        Ok(())
    }

    async fn invalidate(&self, counter_key: &str) {
        self.record(format!("invalidate {counter_key}"));
        let _removed = lock(&self.values).remove(counter_key);
    }
}

fn reserved() -> ReservationHandle {
    let entry = ReservationEntry {
        counter_key: Some(COUNTER.to_owned()),
        entity_type: CounterEntityType::Key,
        entity_id: "k1".to_owned(),
        reserved_cost: Some(Decimal::from(5_i32)),
        applied_adjustment: Decimal::ZERO,
    };
    ReservationHandle::new(Decimal::from(5_i32), vec![entry], Decimal::ZERO)
}

fn mapping(encoded: &str) -> IndexMap<String, Json> {
    serde_json::from_str(encoded).expect("a litellm_params mapping")
}

#[tokio::test]
async fn a_bound_reservation_stays_on_the_counters_and_open() {
    let counters = RecordingCounters::holding(Decimal::from(5_i32));
    let handle = reserved();
    let binder = RequestReservationBinder {
        reservation: Some(handle.clone()),
    };
    let mut litellm_params = mapping(BOTH_BUCKETS);
    binder.bind(&mut litellm_params);
    release_unbound_reservation(&counters, &handle).await;
    assert!(lock(&counters.calls).is_empty());
    assert_eq!(
        lock(&counters.values).get(COUNTER).copied(),
        Some(Decimal::from(5_i32))
    );
    let state = lock(&handle.0).clone();
    assert!(state.callback_bound);
    assert!(!state.finalized);
}

#[tokio::test]
async fn an_unbound_reservation_is_released_to_zero_and_finalized() {
    let counters = RecordingCounters::holding(Decimal::from(5_i32));
    let handle = reserved();
    release_unbound_reservation(&counters, &handle).await;
    assert_eq!(
        lock(&counters.values).get(COUNTER).copied(),
        Some(Decimal::ZERO)
    );
    assert_eq!(
        lock(&counters.calls).clone(),
        vec![
            format!("scope {:?}", BTreeSet::from([COUNTER.to_owned()])),
            format!("read {COUNTER}"),
            format!("pipeline {COUNTER} -5"),
        ]
    );
    let state = lock(&handle.0).clone();
    assert!(!state.callback_bound);
    assert!(state.finalized);
}

#[rstest]
#[case::with_a_handle(Some(reserved()))]
#[case::without_a_handle(None)]
fn bind_stamps_callback_bound_in_both_metadata_buckets(
    #[case] reservation: Option<ReservationHandle>,
) {
    let mut litellm_params = mapping(BOTH_BUCKETS);
    RequestReservationBinder { reservation }.bind(&mut litellm_params);
    assert_eq!(litellm_params, mapping(BOTH_BOUND));
}
