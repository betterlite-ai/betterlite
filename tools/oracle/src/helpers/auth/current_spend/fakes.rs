use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_auth::current_spend::cache::{CounterBatch, CounterCache};
use betterlite_auth::spend_counter_batch::batch::{CounterValue, SpendCounterBatchError};
use betterlite_auth::spend_reseed::cache::{ActiveSpendBatch, ReseedCache};
use betterlite_auth::spend_reseed::rows::{
    SpendAggregateRow, SpendCounterRows, SpendRow, SpendRowLookup, WindowEntity, WindowSpendRow,
};
use betterlite_auth::spend_reseed::values::ReseedCacheValue;
use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use serde_json::{Number, Value, json};

use super::input::{BatchMode, Input, RowsInput};

#[derive(Debug, thiserror::Error)]
#[error("the scripted store failed")]
pub(crate) struct ScriptedFailure;

// each entry keeps the ttl it was written with, the way LiteLLM's in-memory cache does
pub(crate) type Memory = Arc<Mutex<BTreeMap<String, (ReseedCacheValue, Option<u32>)>>>;

pub(crate) fn amount_json(amount: Decimal) -> Value {
    amount
        .to_f64()
        .and_then(Number::from_f64)
        .map_or(Value::Null, Value::Number)
}

pub(crate) fn memory_json(memory: &Memory, key: &str) -> Value {
    match memory
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(key)
        .map(|(value, _ttl)| value)
    {
        Some(ReseedCacheValue::Number(amount)) => amount_json(*amount),
        Some(ReseedCacheValue::Text(text)) => Value::String(text.clone()),
        Some(ReseedCacheValue::Boolean(_) | ReseedCacheValue::Unreadable) | None => Value::Null,
    }
}

pub(crate) struct ScriptedCache<'input> {
    pub(crate) input: &'input Input,
    pub(crate) memory: Memory,
    pub(crate) set_max_calls: Mutex<Vec<Value>>,
}

impl ScriptedCache<'_> {
    fn redis_answer<Answer>(&self, fails: bool, answer: Answer) -> Result<Answer, ScriptedFailure> {
        if fails {
            return Err(ScriptedFailure);
        }
        Ok(answer)
    }
}

impl ReseedCache for ScriptedCache<'_> {
    type Error = ScriptedFailure;

    fn redis_configured(&self) -> bool {
        self.input.redis.0
    }

    fn redis_get(
        &self,
        _counter_key: &str,
    ) -> impl Future<Output = Result<Option<ReseedCacheValue>, ScriptedFailure>> + Send {
        let stored = self.input.redis_get.clone().map(ReseedCacheValue::Text);
        ready(self.redis_answer(self.input.redis_fails.0, stored))
    }

    fn redis_set_nx(
        &self,
        _counter_key: &str,
        _value: Decimal,
    ) -> impl Future<Output = Result<Option<bool>, ScriptedFailure>> + Send {
        ready(Ok(Some(true)))
    }

    async fn memory_get(
        &self,
        counter_key: &str,
    ) -> Result<Option<ReseedCacheValue>, ScriptedFailure> {
        Ok(self
            .memory
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(counter_key)
            .map(|(value, _ttl)| value.clone()))
    }

    fn memory_set(
        &self,
        counter_key: &str,
        value: Decimal,
    ) -> impl Future<Output = Result<(), ScriptedFailure>> + Send {
        self.memory
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                counter_key.to_owned(),
                (ReseedCacheValue::Number(value), None),
            );
        ready(Ok(()))
    }

    fn increment_local(
        &self,
        _counter_key: &str,
        _increment: Decimal,
    ) -> impl Future<Output = Result<Option<Decimal>, ScriptedFailure>> + Send {
        ready(Ok(None))
    }
}

impl CounterCache for ScriptedCache<'_> {
    fn memory_set_expiring(
        &self,
        key: &str,
        value: Decimal,
        ttl_seconds: u32,
    ) -> Result<(), ScriptedFailure> {
        let entry = (ReseedCacheValue::Number(value), Some(ttl_seconds));
        self.memory
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), entry);
        Ok(())
    }

    async fn redis_set_max(
        &self,
        counter_key: &str,
        value: Decimal,
    ) -> Result<Option<Decimal>, ScriptedFailure> {
        self.set_max_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!([counter_key, amount_json(value)]));
        self.redis_answer(self.input.set_max_fails.0, self.input.set_max)
    }
}

#[derive(Clone, Copy)]
enum BatchState {
    Absent,
    Loaded(Option<Decimal>),
    Forgotten,
}

pub(crate) struct ScriptedBatch(Mutex<BatchState>);

impl ScriptedBatch {
    pub(crate) fn open(input: &Input) -> Self {
        let state = match (input.redis.0, input.batch) {
            (true, BatchMode::Amount) => BatchState::Loaded(Some(input.batch_amount)),
            (true, BatchMode::Miss) => BatchState::Loaded(None),
            (true | false, BatchMode::None) | (false, BatchMode::Miss | BatchMode::Amount) => {
                BatchState::Absent
            }
        };
        Self(Mutex::new(state))
    }

    pub(crate) fn after(&self) -> Value {
        match *self.0.lock().unwrap_or_else(PoisonError::into_inner) {
            BatchState::Loaded(value) => json!({"value": value.map_or(Value::Null, amount_json)}),
            BatchState::Absent | BatchState::Forgotten => Value::Null,
        }
    }
}

impl ActiveSpendBatch for ScriptedBatch {
    fn read(
        &self,
        _counter_key: &str,
    ) -> impl Future<Output = Result<Option<CounterValue>, SpendCounterBatchError>> + Send {
        let answer = match *self.0.lock().unwrap_or_else(PoisonError::into_inner) {
            BatchState::Loaded(Some(amount)) => Some(CounterValue::Amount(amount)),
            BatchState::Loaded(None) => Some(CounterValue::Missing),
            BatchState::Absent | BatchState::Forgotten => None,
        };
        ready(Ok(answer))
    }

    fn record(&self, _counter_key: &str, value: Decimal) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if !matches!(*state, BatchState::Absent) {
            *state = BatchState::Loaded(Some(value));
        }
    }
}

impl CounterBatch for ScriptedBatch {
    fn forget(&self, _counter_key: &str) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if !matches!(*state, BatchState::Absent) {
            *state = BatchState::Forgotten;
        }
    }
}

pub(crate) struct ScriptedRows {
    pub(crate) rows: RowsInput,
    pub(crate) memory: Memory,
    pub(crate) marker_key: String,
    pub(crate) window_start: Option<Timestamp>,
}

impl SpendCounterRows for ScriptedRows {
    type Error = ScriptedFailure;

    fn spend_row(
        &self,
        lookup: SpendRowLookup<'_>,
    ) -> impl Future<Output = Result<Option<SpendRow>, ScriptedFailure>> + Send {
        if self.rows.fails.0 {
            return ready(Err(ScriptedFailure));
        }
        if let Some(marker) = self.rows.reset_marker {
            let entry = (ReseedCacheValue::Number(marker), None);
            self.memory
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(self.marker_key.clone(), entry);
        }
        let spend = match lookup {
            SpendRowLookup::EndUser(_) => self.rows.end_user,
            SpendRowLookup::Key(_)
            | SpendRowLookup::TeamMember { .. }
            | SpendRowLookup::Team(_)
            | SpendRowLookup::User(_)
            | SpendRowLookup::Organization(_)
            | SpendRowLookup::Project(_) => self.rows.primary,
        };
        ready(Ok(spend.map(|amount| SpendRow {
            spend: Some(JsonData::String(amount.to_string())),
        })))
    }

    fn window_row(
        &self,
        _entity: WindowEntity,
        _entity_id: &str,
        _window_duration: &str,
    ) -> impl Future<Output = Result<Option<WindowSpendRow>, ScriptedFailure>> + Send {
        if self.rows.fails.0 {
            return ready(Err(ScriptedFailure));
        }
        let row = self
            .rows
            .window
            .zip(self.window_start)
            .map(|(amount, window_start)| WindowSpendRow {
                window_start,
                spend: Some(JsonData::String(amount.to_string())),
            });
        ready(Ok(row))
    }

    async fn window_log_rows(
        &self,
        _entity: WindowEntity,
        _entity_id: &str,
        _window_start: DateTime<Utc>,
    ) -> Result<Vec<SpendAggregateRow>, ScriptedFailure> {
        if self.rows.fails.0 {
            return Err(ScriptedFailure);
        }
        Ok(Vec::new())
    }
}
