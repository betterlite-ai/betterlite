use std::collections::{BTreeMap, BTreeSet};
use std::mem;
use std::sync::Mutex;
use std::time::{Duration, UNIX_EPOCH};

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_redis::request_batch::lock;
use rust_decimal::Decimal;

use crate::budget_counter::{BudgetCounter, CounterEntityType};
use crate::reservation_admission::counters::AdmissionCounters;
use crate::reservation_admission::engine::AdmissionRequest;
use crate::reservation_settlement::counters::{ScopeHold, SettlementCounters};
use crate::reservation_settlement::error::{CounterReadError, SettlementError};
use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};
use crate::spend_counter_batch::batch::PendingSpendIncrement;
use crate::spend_writes::error::{LayersError, SpendWriteError};

pub(super) const KEY: &str = "spend:key:k1";
pub(super) const WINDOW: &str = "spend:key:k1:window:1d";
pub(super) const USER: &str = "spend:user:u1";

// the window's own start: a counter built outside the counter builder still has to carry one
const WINDOW_START_SECONDS: u64 = 1_704_067_200;

pub(super) fn amount(text: &str) -> Decimal {
    text.parse().expect("a fixture decimal")
}

// a counter that seeds its spend from the auth cache under its own key
pub(super) fn entity(
    counter_key: &str,
    max_budget: &str,
    fallback_spend: &str,
    kind: CounterEntityType,
) -> BudgetCounter {
    BudgetCounter {
        counter_key: counter_key.to_owned(),
        max_budget: amount(max_budget),
        fallback_spend: amount(fallback_spend),
        entity_type: kind,
        entity_id: counter_key.to_owned(),
        source_cache_key: Some(counter_key.to_owned()),
        spend_log_entity_id: None,
        window_duration: None,
        window_start: None,
    }
}

// a window counter whose spend loads from the window row under the entity
pub(super) fn window(counter_key: &str, max_budget: &str) -> BudgetCounter {
    BudgetCounter {
        counter_key: counter_key.to_owned(),
        max_budget: amount(max_budget),
        fallback_spend: Decimal::ZERO,
        entity_type: CounterEntityType::Key,
        entity_id: "k1:1d".to_owned(),
        source_cache_key: None,
        spend_log_entity_id: Some("k1".to_owned()),
        window_duration: Some("1d".to_owned()),
        window_start: UNIX_EPOCH.checked_add(Duration::from_secs(WINDOW_START_SECONDS)),
    }
}

fn counted(value: i64) -> JsonData {
    JsonData::Number(serde_json::Number::from(value))
}

// a token whose metadata opts in or out of throttling, named like the key counter's own key
pub(super) fn token(opted_in: bool) -> UserAPIKeyAuth {
    let mut metadata = BTreeMap::new();
    let _throttle = metadata.insert(
        "throttle_on_budget_exceeded".to_owned(),
        JsonData::Bool(opted_in),
    );
    let mut fields = BTreeMap::new();
    let _token = fields.insert("token".to_owned(), JsonData::String("k1".to_owned()));
    let _tpm = fields.insert("tpm_limit".to_owned(), counted(10));
    let _max_budget = fields.insert("max_budget".to_owned(), counted(10));
    let _spend = fields.insert("spend".to_owned(), counted(0));
    let _user = fields.insert("user_id".to_owned(), JsonData::String("u1".to_owned()));
    let _metadata = fields.insert("metadata".to_owned(), JsonData::Object(metadata));
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).expect("the auth fixture builds")
}

// a fake whose counters hold these spend values and whose auth cache answers these source spends
pub(super) fn seeded(values: &[(&str, &str)], sources: &[(&str, &str)]) -> FakeMemory {
    let mut fake = FakeMemory::default();
    for (counter_key, value) in values {
        let _previous = lock(&fake.values).insert((*counter_key).to_owned(), amount(value));
    }
    for (source_key, value) in sources {
        let _previous = fake.sources.insert((*source_key).to_owned(), amount(value));
    }
    fake
}

pub(super) fn request<'a>(counters: &'a [BudgetCounter], cost: &str) -> AdmissionRequest<'a> {
    AdmissionRequest {
        counters,
        token: None,
        reservation_cost: amount(cost),
        fail_closed_budget_enforcement: false,
        budget_exceeded_status: 422,
        throttle_percentage: None,
    }
}

#[derive(Clone, Default)]
pub(super) struct Calls {
    pub(super) seeds: Vec<(String, String)>,
    pub(super) windows: Vec<String>,
    pub(super) spends: Vec<String>,
    pub(super) cache_reads: Vec<String>,
    pub(super) reservations: Vec<Vec<(String, Decimal)>>,
    pub(super) settlements: Vec<Vec<(String, Decimal)>>,
    pub(super) invalidations: Vec<String>,
}

#[derive(Default)]
pub(super) struct FakeMemory {
    pub(super) values: Mutex<BTreeMap<String, Decimal>>,
    pub(super) sources: BTreeMap<String, Decimal>,
    pub(super) failed_seeds: BTreeSet<String>,
    pub(super) failed_pipelines: BTreeSet<String>,
    pub(super) unreadable: BTreeSet<String>,
    // results the reservation pipeline leaves off its answer, which LiteLLM pads with None
    pub(super) dropped_results: usize,
    pub(super) calls: Mutex<Calls>,
}

impl FakeMemory {
    fn apply(&self, counter_key: &str, increment: Decimal) -> Result<Decimal, SettlementError> {
        let held = lock(&self.values)
            .get(counter_key)
            .copied()
            .unwrap_or(Decimal::ZERO);
        let after = held
            .checked_add(increment)
            .ok_or(SettlementError::Overflow)?;
        let _previous = lock(&self.values).insert(counter_key.to_owned(), after);
        Ok(after)
    }
}

pub(super) fn take_calls(fake: &FakeMemory) -> Calls {
    let mut held = lock(&fake.calls);
    mem::take(&mut *held)
}

pub(super) fn value_of(fake: &FakeMemory, counter_key: &str) -> Option<Decimal> {
    lock(&fake.values).get(counter_key).copied()
}

fn record(pending: &[PendingSpendIncrement]) -> Vec<(String, Decimal)> {
    pending
        .iter()
        .map(|item| (item.counter_key.clone(), item.increment))
        .collect()
}

fn store_failure() -> SettlementError {
    SettlementError::Write(SpendWriteError::from(LayersError::Unavailable))
}

fn seed_failure() -> SpendWriteError {
    SpendWriteError::from(LayersError::Unavailable)
}

pub(super) struct FakeScope;

impl ScopeHold for FakeScope {}

#[async_trait]
impl SettlementCounters for FakeMemory {
    fn enter_counters_batch_scope(
        &self,
        _counter_keys: &BTreeSet<String>,
    ) -> Box<dyn ScopeHold + Send + '_> {
        Box::new(FakeScope)
    }

    async fn read_cache_value(
        &self,
        counter_key: &str,
    ) -> Result<Option<Decimal>, CounterReadError> {
        lock(&self.calls).cache_reads.push(counter_key.to_owned());
        if self.unreadable.contains(counter_key) {
            return Err(CounterReadError::NotNumeric {
                counter_key: counter_key.to_owned(),
            });
        }
        Ok(lock(&self.values).get(counter_key).copied())
    }

    async fn reseed_from_db(&self, _counter_key: &str) -> bool {
        false
    }

    async fn increment_counter(
        &self,
        counter_key: &str,
        increment: Decimal,
    ) -> Result<(), SettlementError> {
        let _applied = self.apply(counter_key, increment)?;
        Ok(())
    }

    async fn increment_pipeline(
        &self,
        pending: &[PendingSpendIncrement],
    ) -> Result<(), SettlementError> {
        lock(&self.calls).settlements.push(record(pending));
        for item in pending {
            let _applied = self.apply(&item.counter_key, item.increment)?;
        }
        Ok(())
    }

    async fn invalidate(&self, counter_key: &str) {
        lock(&self.calls).invalidations.push(counter_key.to_owned());
        let _removed = lock(&self.values).remove(counter_key);
    }
}

#[async_trait]
impl AdmissionCounters for FakeMemory {
    async fn seed_source(
        &self,
        counter_key: &str,
        source_cache_key: &str,
    ) -> Result<(), SpendWriteError> {
        lock(&self.calls)
            .seeds
            .push((counter_key.to_owned(), source_cache_key.to_owned()));
        if self.failed_seeds.contains(counter_key) {
            return Err(seed_failure());
        }
        if lock(&self.values).contains_key(counter_key) {
            return Ok(());
        }
        let base_spend = self
            .sources
            .get(source_cache_key)
            .copied()
            .unwrap_or(Decimal::ZERO);
        if base_spend > Decimal::ZERO {
            let held = lock(&self.values)
                .get(counter_key)
                .copied()
                .unwrap_or(Decimal::ZERO);
            let _previous =
                lock(&self.values).insert(counter_key.to_owned(), held.saturating_add(base_spend));
        }
        Ok(())
    }

    async fn seed_window(&self, counter: &BudgetCounter) -> Result<bool, SpendWriteError> {
        lock(&self.calls).windows.push(counter.counter_key.clone());
        if self.failed_seeds.contains(&counter.counter_key) {
            return Err(seed_failure());
        }
        Ok(lock(&self.values).contains_key(&counter.counter_key))
    }

    async fn current_spend(&self, counter_key: &str, fallback_spend: Decimal) -> Decimal {
        lock(&self.calls).spends.push(counter_key.to_owned());
        lock(&self.values)
            .get(counter_key)
            .copied()
            .unwrap_or(fallback_spend)
    }

    async fn reserve_pipeline(
        &self,
        pending: &[PendingSpendIncrement],
    ) -> Result<Vec<Option<Decimal>>, SettlementError> {
        lock(&self.calls).reservations.push(record(pending));
        if pending
            .iter()
            .any(|item| self.failed_pipelines.contains(&item.counter_key))
        {
            return Err(store_failure());
        }
        let mut counted = Vec::new();
        for item in pending {
            counted.push(Some(self.apply(&item.counter_key, item.increment)?));
        }
        counted.truncate(counted.len().saturating_sub(self.dropped_results));
        Ok(counted)
    }
}
