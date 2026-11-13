use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_context::local_ttl_cache::{CacheValue, LocalTtlCache};
use rust_decimal::Decimal;
use tokio::sync::Notify;
use tokio_util::task::TaskTracker;

use crate::budget_increment::dual::DualWindowCache;
use crate::budget_increment::error::IncrementError;
use crate::budget_increment::increment::BudgetIncrement;
use crate::budget_increment::queue::QueuedIncrement;
use crate::budget_increment::window::{BudgetConfig, WindowRedis};
use crate::routing_value_store::Clock;

#[derive(Clone)]
pub(super) struct TestClock(Arc<Mutex<f64>>);

impl TestClock {
    pub(super) fn new(seconds: f64) -> Self {
        Self(Arc::new(Mutex::new(seconds)))
    }

    pub(super) fn set(&self, seconds: f64) {
        *self.0.lock().unwrap() = seconds;
    }

    pub(super) fn clock(&self) -> Clock {
        let source = Arc::clone(&self.0);
        Arc::new(move || *source.lock().unwrap())
    }
}

#[derive(Clone, Default)]
pub(super) struct FakeRedis {
    starts: Arc<Mutex<BTreeMap<String, f64>>>,
    start_writes: Arc<Mutex<Vec<(String, f64, Duration)>>>,
    spend_writes: Arc<Mutex<Vec<(String, Decimal, Duration)>>>,
    read_keys: Arc<Mutex<Vec<String>>>,
    batches: Arc<Mutex<Vec<Vec<QueuedIncrement>>>>,
    fail: Arc<AtomicBool>,
    block: Arc<AtomicBool>,
    read_error: Arc<AtomicBool>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
}

impl FakeRedis {
    pub(super) fn seed_start(&self, key: &str, seconds: f64) {
        self.starts.lock().unwrap().insert(key.to_owned(), seconds);
    }

    pub(super) fn start_writes(&self) -> Vec<(String, f64, Duration)> {
        self.start_writes.lock().unwrap().clone()
    }

    pub(super) fn spend_writes(&self) -> Vec<(String, Decimal, Duration)> {
        self.spend_writes.lock().unwrap().clone()
    }

    pub(super) fn read_keys(&self) -> Vec<String> {
        self.read_keys.lock().unwrap().clone()
    }

    pub(super) fn batches(&self) -> Vec<Vec<QueuedIncrement>> {
        self.batches.lock().unwrap().clone()
    }

    pub(super) fn fail(&self, fail: bool) {
        self.fail.store(fail, Ordering::SeqCst);
    }

    pub(super) fn block(&self, block: bool) {
        self.block.store(block, Ordering::SeqCst);
    }

    pub(super) fn read_error(&self, error: bool) {
        self.read_error.store(error, Ordering::SeqCst);
    }

    pub(super) fn entered(&self) -> &Notify {
        &self.entered
    }

    pub(super) fn release(&self) {
        self.release.notify_waiters();
    }
}

#[async_trait]
impl WindowRedis for FakeRedis {
    async fn read_start_time(&self, key: &str) -> Result<Option<f64>, IncrementError> {
        self.read_keys.lock().unwrap().push(key.to_owned());
        if self.read_error.load(Ordering::SeqCst) {
            return Err(IncrementError::InvalidCacheValue {
                key: key.to_owned(),
            });
        }
        Ok(self.starts.lock().unwrap().get(key).copied())
    }

    async fn write_start_time(&self, key: &str, start_time: f64, ttl: Duration) {
        self.start_writes
            .lock()
            .unwrap()
            .push((key.to_owned(), start_time, ttl));
        self.starts
            .lock()
            .unwrap()
            .insert(key.to_owned(), start_time);
    }

    async fn write_spend(&self, key: &str, cost: Decimal, ttl: Duration) {
        self.spend_writes
            .lock()
            .unwrap()
            .push((key.to_owned(), cost, ttl));
    }

    async fn write_increments(&self, operations: &[QueuedIncrement]) -> bool {
        if self.block.load(Ordering::SeqCst) {
            self.entered.notify_one();
            self.release.notified().await;
        }
        if self.fail.load(Ordering::SeqCst) {
            return false;
        }
        self.batches.lock().unwrap().push(operations.to_vec());
        true
    }
}

pub(super) struct Fixture {
    pub(super) increment: Arc<BudgetIncrement<DualWindowCache<FakeRedis>>>,
    pub(super) cache: Arc<DualWindowCache<FakeRedis>>,
    pub(super) memory: Arc<Mutex<LocalTtlCache>>,
    pub(super) redis: Option<FakeRedis>,
    pub(super) clock: TestClock,
}

impl Fixture {
    pub(super) fn new(now: f64) -> Self {
        let clock = TestClock::new(now);
        let redis = FakeRedis::default();
        Self::build(&clock, Some(redis))
    }

    pub(super) fn memory_only(now: f64) -> Self {
        let clock = TestClock::new(now);
        Self::build(&clock, None)
    }

    fn build(clock: &TestClock, redis: Option<FakeRedis>) -> Self {
        let source = Arc::clone(&clock.0);
        let memory = Arc::new(Mutex::new(
            LocalTtlCache::with_clock(
                Some(200),
                Some(600.0_f64),
                None,
                1024,
                Arc::new(move || *source.lock().unwrap()),
            )
            .unwrap(),
        ));
        let cache = Arc::new(DualWindowCache {
            memory: Arc::clone(&memory),
            redis: redis.clone(),
        });
        let increment = Arc::new(BudgetIncrement {
            cache: Arc::clone(&cache),
            clock: clock.clock(),
            queue: Arc::default(),
            flush_lock: Arc::default(),
        });
        Self {
            increment,
            cache,
            memory,
            redis,
            clock: clock.clone(),
        }
    }

    pub(super) fn at(&self, now: f64) {
        self.clock.set(now);
    }

    pub(super) fn redis(&self) -> &FakeRedis {
        self.redis.as_ref().unwrap()
    }

    pub(super) fn memory_value(&self, key: &str) -> Option<CacheValue> {
        self.memory.lock().unwrap().get(key).unwrap()
    }

    pub(super) fn memory_expiry(&self, key: &str) -> Option<f64> {
        self.memory
            .lock()
            .unwrap()
            .ttl_entries()
            .into_iter()
            .find(|(name, _)| name == key)
            .map(|(_, expiry)| expiry)
    }
}

pub(super) fn budget(seconds: u64) -> BudgetConfig {
    BudgetConfig {
        max_budget: Some(Decimal::from(100)),
        budget_duration: Some(Duration::from_secs(seconds)),
    }
}

pub(super) fn queued(key: &str, value: Decimal, ttl: u64) -> QueuedIncrement {
    QueuedIncrement {
        key: key.to_owned(),
        increment_value: value,
        ttl: Duration::from_secs(ttl),
    }
}

pub(super) async fn flush(fixture: &Fixture) -> bool {
    let tasks = TaskTracker::new();
    fixture.increment.flush_increments(&tasks).await
}
