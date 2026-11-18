use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use betterlite_auth::budget_primitives::window_start::DurationParser;
use betterlite_auth::model_budget_limiter::clock::SystemClock as BudgetSystemClock;
use betterlite_auth::spend_counter_batch::batch::{CounterValue, SpendCounterBatchError};
use betterlite_auth::spend_reseed::cache::{ActiveSpendBatch, ReseedCache};
use betterlite_auth::spend_reseed::locks::LockRegistry;
use betterlite_auth::spend_reseed::reseed::SpendCounterReseed;
use betterlite_auth::spend_reseed::values::ReseedCacheValue;
use betterlite_db::budget_permission_rows::Timestamp;
use betterlite_db::database_failures::gate::{
    DbLookupStallTracker, LoopBoundSemaphore, SystemClock,
};
use betterlite_oracle_helper_api::error::HelperError;
use chrono::{DateTime, NaiveDateTime, Utc};
use rust_decimal::Decimal;
use rust_decimal::prelude::{FromPrimitive as _, ToPrimitive as _};
use serde_json::{Value, json};
use tokio_util::task::TaskTracker;

use super::input::{Answer, CacheInput, SetNx, Word};
use super::rows::FakeRows;

#[derive(Debug, thiserror::Error)]
#[error("the fake store failed")]
pub(crate) struct FakeError;

pub(crate) fn failure<Error>(error: Error) -> Box<HelperError>
where
    Error: std::fmt::Display,
{
    Box::new(HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

pub(crate) fn amount_json(amount: Decimal) -> Value {
    amount
        .to_f64()
        .and_then(serde_json::Number::from_f64)
        .map_or(Value::Null, Value::Number)
}

pub(crate) fn timestamp(text: &str) -> Result<Timestamp, Box<HelperError>> {
    if let Ok(aware) = DateTime::parse_from_rfc3339(text) {
        return Ok(Timestamp::Aware(aware));
    }
    NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%.f")
        .map(Timestamp::Naive)
        .map_err(failure)
}

pub(crate) struct FakeCache {
    state: Mutex<CacheState>,
    redis: bool,
    set_nx: SetNx,
    memory_set_fails: bool,
}

struct CacheState {
    memory: Option<f64>,
    answers: VecDeque<Answer>,
    sets: Vec<(String, Decimal)>,
}

impl FakeCache {
    pub(crate) fn build(input: &CacheInput) -> Self {
        Self {
            state: Mutex::new(CacheState {
                memory: input.memory,
                answers: input.answers.iter().cloned().collect(),
                sets: Vec::new(),
            }),
            redis: input.redis,
            set_nx: input.set_nx,
            memory_set_fails: input.memory_set_fails,
        }
    }

    fn stored_amount(&self) -> Option<Decimal> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .memory
            .and_then(Decimal::from_f64)
    }

    pub(crate) fn memory_json(&self) -> Value {
        let Some(amount) = self.stored_amount() else {
            return Value::Null;
        };
        amount_json(amount)
    }

    pub(crate) fn sets_json(&self) -> Value {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        Value::Array(
            state
                .sets
                .iter()
                .map(|(key, amount)| json!([key, amount_json(*amount)]))
                .collect(),
        )
    }
}

impl ReseedCache for FakeCache {
    type Error = FakeError;

    fn redis_configured(&self) -> bool {
        self.redis
    }

    async fn redis_get(&self, _counter_key: &str) -> Result<Option<ReseedCacheValue>, FakeError> {
        if !self.redis {
            return Ok(None);
        }
        let answer = self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .answers
            .pop_front();
        match answer {
            Some(Answer::Amount(value)) => {
                Ok(Decimal::from_f64(value).map(ReseedCacheValue::Number))
            }
            Some(Answer::Word(Word::Fail)) => Err(FakeError),
            Some(Answer::Word(Word::Miss)) | None => Ok(None),
        }
    }

    async fn redis_set_nx(
        &self,
        counter_key: &str,
        value: Decimal,
    ) -> Result<Option<bool>, FakeError> {
        if !self.redis {
            return Ok(None);
        }
        if matches!(self.set_nx, SetNx::Fail) {
            return Err(FakeError);
        }
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .sets
            .push((counter_key.to_owned(), value));
        Ok(Some(matches!(self.set_nx, SetNx::Stored)))
    }

    async fn memory_get(&self, _counter_key: &str) -> Result<Option<ReseedCacheValue>, FakeError> {
        let Some(amount) = self.stored_amount() else {
            return Ok(None);
        };
        Ok(Some(ReseedCacheValue::Number(amount)))
    }

    async fn memory_set(&self, _counter_key: &str, value: Decimal) -> Result<(), FakeError> {
        if self.memory_set_fails {
            return Err(FakeError);
        }
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .memory = value.to_f64();
        Ok(())
    }

    async fn increment_local(
        &self,
        _counter_key: &str,
        increment: Decimal,
    ) -> Result<Option<Decimal>, FakeError> {
        // LiteLLM's increment swallows a memory write failure and leaves the counter alone
        if self.memory_set_fails {
            return Ok(None);
        }
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let base = state
            .memory
            .and_then(Decimal::from_f64)
            .unwrap_or(Decimal::ZERO);
        let next = base.checked_add(increment).ok_or(FakeError)?;
        state.memory = next.to_f64();
        Ok(Some(next))
    }
}

pub(crate) struct FakeBatch {
    answer: Option<Answer>,
    recorded: Mutex<Option<Decimal>>,
}

impl FakeBatch {
    pub(crate) fn new(answer: Option<Answer>) -> Self {
        Self {
            answer,
            recorded: Mutex::new(None),
        }
    }

    pub(crate) fn after(&self) -> Value {
        let recorded = *self.recorded.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(amount) = recorded {
            return amount_json(amount);
        }
        match &self.answer {
            Some(Answer::Amount(value)) => {
                Decimal::from_f64(*value).map_or(Value::Null, amount_json)
            }
            Some(Answer::Word(Word::Miss | Word::Fail)) | None => Value::Null,
        }
    }
}

impl ActiveSpendBatch for FakeBatch {
    async fn read(
        &self,
        _counter_key: &str,
    ) -> Result<Option<CounterValue>, SpendCounterBatchError> {
        match &self.answer {
            Some(Answer::Amount(value)) => Decimal::from_f64(*value)
                .map(|amount| Some(CounterValue::Amount(amount)))
                .ok_or(SpendCounterBatchError::Cancelled),
            Some(Answer::Word(Word::Miss)) => Ok(Some(CounterValue::Missing)),
            Some(Answer::Word(Word::Fail)) | None => Ok(None),
        }
    }

    fn record(&self, _counter_key: &str, value: Decimal) {
        *self.recorded.lock().unwrap_or_else(PoisonError::into_inner) = Some(value);
    }
}

pub(crate) fn counter(rows: Option<Arc<FakeRows>>) -> SpendCounterReseed<FakeRows> {
    SpendCounterReseed {
        rows,
        locks: LockRegistry::from(10_000),
        clock: Box::new(BudgetSystemClock {
            started: Instant::now(),
        }),
        durations: Box::new(FixedDurations),
        tasks: TaskTracker::new(),
        gate: Arc::new(LoopBoundSemaphore {
            value: 25,
            slot: Mutex::new(None),
        }),
        stall_tracker: Arc::new(DbLookupStallTracker {
            clock: SystemClock,
            last_hit: Mutex::new(None),
        }),
        deadline_seconds: 5.0,
    }
}

struct FixedDurations;

impl DurationParser for FixedDurations {
    fn seconds(&self, spec: &str, _now: DateTime<Utc>) -> Option<i64> {
        match spec {
            "1h" => Some(3_600),
            "1d" => Some(86_400),
            _ => None,
        }
    }
}
