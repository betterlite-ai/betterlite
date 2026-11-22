use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use async_trait::async_trait;
use betterlite_auth::model_budget_values::amount_text::amount_from_text;
use betterlite_auth::reservation_admission::counters::{AdmissionCounters, WindowSeed};
use betterlite_auth::reservation_settlement::counters::{ScopeHold, SettlementCounters};
use betterlite_auth::reservation_settlement::error::{CounterReadError, SettlementError};
use betterlite_auth::spend_counter_batch::batch::PendingSpendIncrement;
use betterlite_auth::spend_writes::error::{LayersError, SpendWriteError};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_redis::request_batch::lock;
use rust_decimal::Decimal;

use super::super::invalid;

// the helper's in-memory spend counter: the case seeds its values, names the keys whose reads raise, and the recorded pipelines and invalidations come back out in the output
pub(crate) struct ScriptedMemory {
    pub(crate) values: Mutex<BTreeMap<String, Decimal>>,
    sources: BTreeMap<String, Decimal>,
    failures: BTreeSet<String>,
    pipeline_fails: bool,
    pub(crate) pipelines: Mutex<Vec<Vec<(String, Decimal)>>>,
    pub(crate) invalidated: Mutex<Vec<String>>,
}

impl ScriptedMemory {
    pub(crate) fn new(
        memory: &BTreeMap<String, serde_json::Number>,
        sources: &BTreeMap<String, serde_json::Number>,
        failures: &BTreeSet<String>,
        pipeline_fails: bool,
    ) -> Result<Self, Box<HelperError>> {
        let mut values = BTreeMap::new();
        for (key, number) in memory {
            let Some(amount) = amount_from_text(&number.to_string()) else {
                return Err(invalid(format!("memory value for {key} is not a number")));
            };
            let _previous = values.insert(key.clone(), amount);
        }
        let mut base_spends = BTreeMap::new();
        for (key, number) in sources {
            let Some(amount) = amount_from_text(&number.to_string()) else {
                return Err(invalid(format!("source value for {key} is not a number")));
            };
            let _previous = base_spends.insert(key.clone(), amount);
        }
        Ok(Self {
            values: Mutex::new(values),
            sources: base_spends,
            failures: failures.clone(),
            pipeline_fails,
            pipelines: Mutex::new(Vec::new()),
            invalidated: Mutex::new(Vec::new()),
        })
    }

    fn record(&self, pending: &[PendingSpendIncrement]) {
        let recorded: Vec<(String, Decimal)> = pending
            .iter()
            .map(|item| (item.counter_key.clone(), item.increment))
            .collect();
        lock(&self.pipelines).push(recorded);
    }

    // InMemoryCache.increment_cache: get or zero, add, set; a scripted failing read aborts the whole pipeline
    fn apply(&self, counter_key: &str, increment: Decimal) -> Result<Decimal, SettlementError> {
        if self.failures.contains(counter_key) {
            return Err(SettlementError::Write(SpendWriteError::from(
                LayersError::Unavailable,
            )));
        }
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

pub(super) struct FakeScope;

impl ScopeHold for FakeScope {}

#[async_trait]
impl SettlementCounters for ScriptedMemory {
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
        if self.failures.contains(counter_key) {
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
        let _after = self.apply(counter_key, increment)?;
        Ok(())
    }

    async fn increment_pipeline(
        &self,
        pending: &[PendingSpendIncrement],
    ) -> Result<(), SettlementError> {
        self.record(pending);
        if self.pipeline_fails {
            return Err(SettlementError::Write(SpendWriteError::from(
                LayersError::Unavailable,
            )));
        }
        for item in pending {
            let _after = self.apply(&item.counter_key, item.increment)?;
        }
        Ok(())
    }

    async fn invalidate(&self, counter_key: &str) {
        lock(&self.invalidated).push(counter_key.to_owned());
        let _removed = lock(&self.values).remove(counter_key);
    }
}

#[async_trait]
impl AdmissionCounters for ScriptedMemory {
    // _ensure_spend_counter_initialized: a warm counter stays, a cold one loads the source row's spend
    async fn seed_source(
        &self,
        counter_key: &str,
        source_cache_key: &str,
    ) -> Result<(), SpendWriteError> {
        if self.failures.contains(counter_key) {
            return Err(SpendWriteError::from(LayersError::Unavailable));
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
            let _previous = lock(&self.values).insert(counter_key.to_owned(), base_spend);
        }
        Ok(())
    }

    // _ensure_window_spend_counter_initialized: without a DB row a cold window answers false
    async fn seed_window(&self, seed: WindowSeed<'_>) -> Result<bool, SpendWriteError> {
        if self.failures.contains(seed.counter_key) {
            return Err(SpendWriteError::from(LayersError::Unavailable));
        }
        Ok(lock(&self.values).contains_key(seed.counter_key))
    }

    async fn current_spend(&self, counter_key: &str, fallback_spend: Decimal) -> Decimal {
        lock(&self.values)
            .get(counter_key)
            .copied()
            .unwrap_or(fallback_spend)
    }

    // run_spend_counter_pipeline: the recorded call, then each counter's value after its increment
    async fn reserve_pipeline(
        &self,
        pending: &[PendingSpendIncrement],
    ) -> Result<Vec<Option<Decimal>>, SettlementError> {
        self.record(pending);
        if self.pipeline_fails {
            return Err(SettlementError::Write(SpendWriteError::from(
                LayersError::Unavailable,
            )));
        }
        let mut values = Vec::new();
        for item in pending {
            values.push(Some(self.apply(&item.counter_key, item.increment)?));
        }
        Ok(values)
    }
}
