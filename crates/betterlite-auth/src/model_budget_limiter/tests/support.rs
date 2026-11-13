use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_error::error::ErrorClass;
use betterlite_error::error::details::BudgetExceededDetails;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use thiserror::Error;

use crate::budget_primitives::window_start::DurationParser;
use crate::model_budget_limiter::ModelBudgetLimiter;
use crate::model_budget_limiter::cache::ModelBudgetCache;
use crate::model_budget_limiter::clock::BudgetClock;
use crate::model_budget_values::candidates::BedrockCatalog;
use crate::model_budget_values::spend_reads::CachedSpend;

pub(crate) const DAY: u64 = 86_400;

#[derive(Debug, Error)]
#[error("the fake cache failed")]
pub(crate) struct FakeFailure;

pub(crate) struct FixedDurations;

impl DurationParser for FixedDurations {
    fn seconds(&self, spec: &str, _now: DateTime<Utc>) -> Option<i64> {
        let units = [("s", 1), ("m", 60), ("h", 3_600), ("d", 86_400)];
        let (digits, factor) = units
            .iter()
            .find_map(|(suffix, factor)| Some((spec.strip_suffix(suffix)?, *factor)))?;
        digits.parse::<i64>().ok()?.checked_mul(factor)
    }
}

// the team override and the entity resolution each parse the duration once, then the charge parses it
pub(crate) struct FailsAfterTwoCalls {
    pub(crate) calls: AtomicUsize,
}

impl DurationParser for FailsAfterTwoCalls {
    fn seconds(&self, spec: &str, now: DateTime<Utc>) -> Option<i64> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if call < 2 {
            FixedDurations.seconds(spec, now)
        } else {
            None
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct FixedClock {
    pub(crate) now: DateTime<Utc>,
    pub(crate) uptime: Duration,
}

impl BudgetClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.now
    }

    fn uptime(&self) -> Duration {
        self.uptime
    }
}

pub(crate) struct NoBedrock;

impl BedrockCatalog for NoBedrock {
    fn base_model(&self, model: &str) -> String {
        model.to_owned()
    }

    fn prices_as_bedrock(&self, _base_model: &str) -> bool {
        false
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SpendCall {
    pub(crate) duration: Duration,
    pub(crate) spend_key: String,
    pub(crate) start_time_key: String,
    pub(crate) cost: Decimal,
}

pub(crate) struct FakeCache {
    pub(crate) redis: bool,
    pub(crate) redis_values: Mutex<BTreeMap<String, CachedSpend>>,
    pub(crate) memory_values: Mutex<BTreeMap<String, CachedSpend>>,
    pub(crate) counter_ttls: Mutex<Vec<(String, Duration)>>,
    pub(crate) spend_calls: Mutex<Vec<SpendCall>>,
    pub(crate) flushes: AtomicUsize,
    pub(crate) fail_spend: AtomicBool,
}

impl FakeCache {
    pub(crate) fn new() -> Self {
        Self {
            redis: false,
            redis_values: Mutex::new(BTreeMap::new()),
            memory_values: Mutex::new(BTreeMap::new()),
            counter_ttls: Mutex::new(Vec::new()),
            spend_calls: Mutex::new(Vec::new()),
            flushes: AtomicUsize::new(0),
            fail_spend: AtomicBool::new(false),
        }
    }

    pub(crate) fn with_redis() -> Self {
        Self {
            redis: true,
            ..Self::new()
        }
    }

    pub(crate) fn preload(&self, key: &str, amount: Decimal) {
        self.memory_values
            .lock()
            .unwrap()
            .insert(key.to_owned(), CachedSpend::Number(amount));
    }

    pub(crate) fn preload_redis(&self, key: &str, amount: Decimal) {
        self.redis_values
            .lock()
            .unwrap()
            .insert(key.to_owned(), CachedSpend::Number(amount));
    }

    pub(crate) fn memory(&self, key: &str) -> Option<Decimal> {
        match self.memory_values.lock().unwrap().get(key) {
            Some(CachedSpend::Number(amount)) => Some(*amount),
            Some(CachedSpend::Text(_) | CachedSpend::Unreadable) | None => None,
        }
    }

    pub(crate) fn calls(&self) -> Vec<SpendCall> {
        self.spend_calls.lock().unwrap().clone()
    }
}

impl ModelBudgetCache for FakeCache {
    type Error = FakeFailure;

    fn redis_configured(&self) -> bool {
        self.redis
    }

    fn redis_spend(&self, key: &str) -> impl Future<Output = Option<CachedSpend>> + Send {
        ready(self.redis_values.lock().unwrap().get(key).cloned())
    }

    fn memory_spend(&self, key: &str) -> impl Future<Output = Option<CachedSpend>> + Send {
        ready(self.memory_values.lock().unwrap().get(key).cloned())
    }

    fn increment_counter(
        &self,
        key: &str,
        ttl: Duration,
    ) -> impl Future<Output = Option<Decimal>> + Send {
        self.counter_ttls
            .lock()
            .unwrap()
            .push((key.to_owned(), ttl));
        let mut values = self.memory_values.lock().unwrap();
        let current = match values.get(key) {
            Some(CachedSpend::Number(amount)) => *amount,
            Some(CachedSpend::Text(_) | CachedSpend::Unreadable) | None => Decimal::ZERO,
        };
        let next = current.checked_add(Decimal::ONE).unwrap_or(Decimal::MAX);
        values.insert(key.to_owned(), CachedSpend::Number(next));
        drop(values);
        ready(Some(next))
    }

    fn increment_spend(
        &self,
        duration: Duration,
        spend_key: &str,
        start_time_key: &str,
        cost: Decimal,
    ) -> impl Future<Output = Result<(), FakeFailure>> + Send {
        if self.fail_spend.load(Ordering::SeqCst) {
            return ready(Err(FakeFailure));
        }
        self.spend_calls.lock().unwrap().push(SpendCall {
            duration,
            spend_key: spend_key.to_owned(),
            start_time_key: start_time_key.to_owned(),
            cost,
        });
        let mut values = self.memory_values.lock().unwrap();
        let current = match values.get(spend_key) {
            Some(CachedSpend::Number(amount)) => *amount,
            Some(CachedSpend::Text(_) | CachedSpend::Unreadable) | None => Decimal::ZERO,
        };
        let next = current.checked_add(cost).unwrap_or(Decimal::MAX);
        values.insert(spend_key.to_owned(), CachedSpend::Number(next));
        drop(values);
        if self.redis {
            self.redis_values
                .lock()
                .unwrap()
                .insert(spend_key.to_owned(), CachedSpend::Number(next));
        }
        ready(Ok(()))
    }

    fn flush_increments(&self) -> impl Future<Output = ()> + Send {
        self.flushes.fetch_add(1, Ordering::SeqCst);
        ready(())
    }
}

pub(crate) fn limiter(cache: FakeCache) -> ModelBudgetLimiter<FakeCache> {
    limiter_at(cache, Duration::ZERO)
}

pub(crate) fn limiter_at(cache: FakeCache, uptime: Duration) -> ModelBudgetLimiter<FakeCache> {
    ModelBudgetLimiter {
        cache,
        durations: Box::new(FixedDurations),
        catalog: Box::new(NoBedrock),
        clock: Box::new(FixedClock {
            now: DateTime::UNIX_EPOCH,
            uptime,
        }),
        budget_exceeded_status: 422,
    }
}

pub(crate) fn limiter_with(
    cache: FakeCache,
    durations: Box<dyn DurationParser + Send + Sync>,
) -> ModelBudgetLimiter<FakeCache> {
    ModelBudgetLimiter {
        cache,
        durations,
        catalog: Box::new(NoBedrock),
        clock: Box::new(FixedClock {
            now: DateTime::UNIX_EPOCH,
            uptime: Duration::ZERO,
        }),
        budget_exceeded_status: 422,
    }
}

pub(crate) fn limiter_with_status(
    cache: FakeCache,
    budget_exceeded_status: u16,
) -> ModelBudgetLimiter<FakeCache> {
    ModelBudgetLimiter {
        cache,
        durations: Box::new(FixedDurations),
        catalog: Box::new(NoBedrock),
        clock: Box::new(FixedClock {
            now: DateTime::UNIX_EPOCH,
            uptime: Duration::ZERO,
        }),
        budget_exceeded_status,
    }
}

pub(crate) fn budgets(text: &str) -> BTreeMap<String, JsonData> {
    serde_json::from_str(text).unwrap()
}

pub(crate) fn amount(text: &str) -> Decimal {
    text.parse().unwrap()
}

pub(crate) fn exceeded(error: ErrorClass) -> Box<BudgetExceededDetails> {
    let ErrorClass::BudgetExceeded(details) = error else {
        panic!("expected a budget exceeded error, got {error:?}");
    };
    details
}
