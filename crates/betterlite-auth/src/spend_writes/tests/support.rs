use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::future::{Future, ready};
use std::io::{Error as IoError, ErrorKind};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_cache::backend::BackendError;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::dual_cache::{DualCache, DualCacheSettings};
use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use betterlite_db::database_failures::gate::{
    DbLookupStallTracker, LoopBoundSemaphore, SystemClock,
};
use betterlite_db::spend_rows::rows::{
    SpendCounterRows as DbSpendCounterRows, SpendRow as DbSpendRow, SpendRowLookup,
};
use betterlite_models::decisions::Json;
use betterlite_redis::batch::result::{Settled, SettledHook};
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::redis_cache::error::RedisCacheError;
use betterlite_redis::redis_client_reuse::breaker::BreakerOpen;
use betterlite_redis::redis_live_client::LiveClientError;
use chrono::{DateTime, FixedOffset, TimeZone as _, Utc};
use redis::RedisError;
use rust_decimal::Decimal;
use tokio_util::task::TaskTracker;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing::{Event, Level};
use tracing_subscriber::layer::{Context, SubscriberExt as _};

use crate::auth_cache::{AuthCache, gateway_cache};
use crate::budget_counter::CounterEntityType;
use crate::budget_primitives::window_start::DurationParser;
use crate::model_budget_limiter::clock::BudgetClock;
use crate::spend_counter_batch::batch::{
    CounterValue, PendingSpendIncrement, SpendCounterBatchError,
};
use crate::spend_reseed::cache::dual::ReseedCacheError;
use crate::spend_reseed::cache::{ReseedCache, WindowReseedCache};
use crate::spend_reseed::locks::LockRegistry;
use crate::spend_reseed::reseed::SpendCounterReseed;
use crate::spend_reseed::rows::{
    SpendAggregateRow, SpendCounterRows, WindowEntity, WindowSpendRow,
};
use crate::spend_reseed::values::ReseedCacheValue;
use crate::spend_reseed::windows::WindowReseed;
use crate::spend_writes::SpendCounterWrites;
use crate::spend_writes::batch::CounterBatch;
use crate::spend_writes::error::LayersError;
use crate::spend_writes::layers::{CounterLayers, CounterWriteBatch, PostCallBatch};
use crate::spend_writes::source::SourceSpendCache;

type CacheResult<T> = Result<T, ReseedCacheError>;

pub(crate) const KEY: &str = "spend:key:k1";
pub(crate) const OTHER_KEY: &str = "spend:key:k2";

pub(crate) fn decimal(text: &str) -> Decimal {
    text.parse().expect("a fixture decimal parses")
}

// the auth cache and the source cache hold betterlite_models Json, whose shape mirrors LiteLLM's rows
pub(crate) fn source_json(text: &str) -> Json {
    serde_json::from_str(text).expect("a fixture JSON value parses")
}

pub(crate) fn json_data(text: &str) -> JsonData {
    serde_json::from_str(text).expect("a fixture database value parses")
}

pub(crate) fn aware(text: &str) -> Timestamp {
    let parsed = DateTime::<FixedOffset>::parse_from_rfc3339(text)
        .expect("a fixture offset timestamp parses");
    Timestamp::Aware(parsed)
}

pub(crate) fn window_row(window_start: Timestamp, spend: &str) -> WindowSpendRow {
    WindowSpendRow {
        window_start,
        spend: Some(json_data(spend)),
    }
}

pub(crate) fn increment(counter_key: &str, amount: &str) -> PendingSpendIncrement {
    let increment = decimal(amount);
    PendingSpendIncrement {
        counter_key: counter_key.to_owned(),
        increment,
    }
}

pub(crate) fn window_request<'query>(
    entity_type: &'query CounterEntityType,
    entity_id: &'query str,
) -> WindowReseed<'query> {
    WindowReseed {
        counter_key: KEY,
        entity_type,
        entity_id,
        window_duration: Some("1d"),
        window_start: aware("2026-11-18T12:00:00+00:00"),
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the fake store failed")]
pub(crate) struct FakeError;

#[derive(Clone, Default)]
pub(crate) struct Journal(Arc<Mutex<Vec<String>>>);

impl Journal {
    pub(crate) fn events(&self) -> Vec<String> {
        self.0.lock().expect("the fake journal lock").clone()
    }

    pub(crate) fn records(&self, event: &str) -> bool {
        self.events().iter().any(|entry| entry == event)
    }

    pub(crate) fn count(&self, prefix: &str) -> usize {
        let events = self.events();
        events
            .iter()
            .filter(|entry| entry.starts_with(prefix))
            .count()
    }

    fn push(&self, event: String) {
        self.0.lock().expect("the fake journal lock").push(event);
    }
}

pub(crate) struct FakeRows {
    pub(crate) spend: Mutex<Option<Decimal>>,
    pub(crate) window: Mutex<Option<WindowSpendRow>>,
    pub(crate) spend_reads: AtomicUsize,
    pub(crate) window_reads: AtomicUsize,
    pub(crate) fails: AtomicBool,
}

impl FakeRows {
    pub(crate) fn cold() -> Self {
        Self {
            spend: Mutex::new(None),
            window: Mutex::new(None),
            spend_reads: AtomicUsize::new(0),
            window_reads: AtomicUsize::new(0),
            fails: AtomicBool::new(false),
        }
    }

    pub(crate) fn hold_spend(&self, amount: Decimal) {
        *self.spend.lock().expect("the fake spend row lock") = Some(amount);
    }

    pub(crate) fn fail(&self) {
        self.fails.store(true, Ordering::Relaxed);
    }

    pub(crate) fn hold_window(&self, row: WindowSpendRow) {
        *self.window.lock().expect("the fake window row lock") = Some(row);
    }

    pub(crate) fn spend_reads(&self) -> usize {
        self.spend_reads.load(Ordering::Relaxed)
    }

    pub(crate) fn window_reads(&self) -> usize {
        self.window_reads.load(Ordering::Relaxed)
    }
}

impl DbSpendCounterRows for FakeRows {
    type Error = FakeError;

    fn spend_row(
        &self,
        _lookup: SpendRowLookup<'_>,
    ) -> impl Future<Output = Result<Option<DbSpendRow>, FakeError>> + Send {
        self.spend_reads.fetch_add(1, Ordering::Relaxed);
        if self.fails.load(Ordering::Relaxed) {
            return ready(Err(FakeError));
        }
        let spend = *self.spend.lock().expect("the fake spend row lock");
        ready(Ok(spend.map(|amount| DbSpendRow {
            spend: Some(amount),
        })))
    }
}

impl SpendCounterRows for FakeRows {
    type Error = FakeError;

    fn window_row(
        &self,
        _entity: WindowEntity,
        _entity_id: &str,
        _window_duration: &str,
    ) -> impl Future<Output = Result<Option<WindowSpendRow>, FakeError>> + Send {
        self.window_reads.fetch_add(1, Ordering::Relaxed);
        if self.fails.load(Ordering::Relaxed) {
            return ready(Err(FakeError));
        }
        let row = self
            .window
            .lock()
            .expect("the fake window row lock")
            .clone();
        ready(Ok(row))
    }

    fn window_log_rows(
        &self,
        _entity: WindowEntity,
        _entity_id: &str,
        _window_start: DateTime<Utc>,
    ) -> impl Future<Output = Result<Vec<SpendAggregateRow>, FakeError>> + Send {
        ready(Ok(Vec::new()))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Failure {
    Breaker,
    Timeout,
    Unavailable,
    Memory,
}

pub(crate) fn breaker_redis_error() -> RedisCacheError {
    RedisCacheError::Breaker(BreakerOpen {
        name: "spend-writes",
    })
}

pub(crate) fn timed_out_live() -> LiveClientError {
    let source = IoError::from(ErrorKind::TimedOut);
    LiveClientError::Redis(RedisError::from(source))
}

pub(crate) fn timeout_redis_error() -> RedisCacheError {
    RedisCacheError::LiveClient(timed_out_live())
}

pub(crate) fn layers_failure(failure: Failure) -> LayersError {
    match failure {
        Failure::Breaker => LayersError::Redis(breaker_redis_error()),
        Failure::Timeout => LayersError::Redis(timeout_redis_error()),
        Failure::Unavailable => LayersError::Unavailable,
        Failure::Memory => LayersError::Memory(BackendError::Unsupported("the fake tier")),
    }
}

pub(crate) fn reseed_failure(failure: Failure) -> ReseedCacheError {
    match failure {
        Failure::Breaker => ReseedCacheError::Redis(breaker_redis_error()),
        Failure::Timeout => ReseedCacheError::Redis(timeout_redis_error()),
        Failure::Unavailable => ReseedCacheError::Unavailable,
        Failure::Memory => ReseedCacheError::Memory(BackendError::Unsupported("the fake tier")),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) enum Fault {
    RedisGet,
    MemoryGet,
    MemorySet,
    IncrementLocal,
    IncrementRedis,
    RefreshTtl,
    DeleteRedis,
    DeleteMemory,
    SetMemoryValue,
    Pipeline,
}

pub(crate) struct Faults(Mutex<BTreeSet<Fault>>);

impl Faults {
    pub(crate) fn add(&self, fault: Fault) {
        self.0.lock().expect("the fake fault lock").insert(fault);
    }

    fn has(&self, fault: Fault) -> bool {
        self.0.lock().expect("the fake fault lock").contains(&fault)
    }
}

pub(crate) struct FakeLayers {
    pub(crate) redis_configured: bool,
    pub(crate) redis: Mutex<Option<ReseedCacheValue>>,
    pub(crate) memory: Mutex<BTreeMap<String, ReseedCacheValue>>,
    pub(crate) increments: Mutex<BTreeMap<String, Decimal>>,
    pub(crate) journal: Journal,
    pub(crate) faults: Faults,
    pub(crate) failure: Mutex<Failure>,
    pub(crate) pipeline_answer: Mutex<Option<Vec<Decimal>>>,
    pub(crate) refresh_answer: Mutex<bool>,
    pub(crate) post_call_batch: Mutex<Option<Arc<FakePostCall>>>,
}

impl FakeLayers {
    pub(crate) fn new(redis_configured: bool, journal: Journal) -> Self {
        Self {
            redis_configured,
            redis: Mutex::new(None),
            memory: Mutex::new(BTreeMap::new()),
            increments: Mutex::new(BTreeMap::new()),
            journal,
            faults: Faults(Mutex::new(BTreeSet::new())),
            failure: Mutex::new(Failure::Breaker),
            pipeline_answer: Mutex::new(None),
            refresh_answer: Mutex::new(true),
            post_call_batch: Mutex::new(None),
        }
    }

    pub(crate) fn put_memory(&self, counter_key: &str, value: ReseedCacheValue) {
        let mut memory = self.memory.lock().expect("the fake memory lock");
        memory.insert(counter_key.to_owned(), value);
    }

    pub(crate) fn memory_value(&self, counter_key: &str) -> Option<ReseedCacheValue> {
        let memory = self.memory.lock().expect("the fake memory lock");
        memory.get(counter_key).cloned()
    }

    pub(crate) fn put_redis(&self, value: ReseedCacheValue) {
        *self.redis.lock().expect("the fake redis lock") = Some(value);
    }

    pub(crate) fn redis_value(&self) -> Option<ReseedCacheValue> {
        self.redis.lock().expect("the fake redis lock").clone()
    }

    pub(crate) fn increment_total(&self, counter_key: &str) -> Option<Decimal> {
        let increments = self.increments.lock().expect("the fake increment lock");
        increments.get(counter_key).copied()
    }

    pub(crate) fn answer_pipeline(&self, amounts: Vec<Decimal>) {
        *self.pipeline_answer.lock().expect("the fake pipeline lock") = Some(amounts);
    }

    pub(crate) fn refresh(&self, refreshed: bool) {
        *self.refresh_answer.lock().expect("the fake refresh lock") = refreshed;
    }

    pub(crate) fn serve_post_call(&self, batch: Arc<FakePostCall>) {
        *self
            .post_call_batch
            .lock()
            .expect("the fake post-call lock") = Some(batch);
    }

    fn failure_now(&self) -> Failure {
        *self.failure.lock().expect("the fake failure lock")
    }

    fn faulted<Wrapped>(&self, fault: Fault, wrap: fn(Failure) -> Wrapped) -> Option<Wrapped> {
        if !self.faults.has(fault) {
            return None;
        }
        Some(wrap(self.failure_now()))
    }

    fn fatal(&self, fault: Fault) -> Option<LayersError> {
        self.faulted(fault, layers_failure)
    }

    fn cache_fatal(&self, fault: Fault) -> Option<ReseedCacheError> {
        self.faulted(fault, reseed_failure)
    }

    fn begin(&self, event: String, fault: Fault) -> Result<(), LayersError> {
        self.journal.push(event);
        self.fatal(fault).map_or(Ok(()), Err)
    }

    fn begin_cache(&self, event: String, fault: Fault) -> Result<(), ReseedCacheError> {
        self.journal.push(event);
        self.cache_fatal(fault).map_or(Ok(()), Err)
    }

    // the journal notes the attempt before the fault fires, so a failed call still shows in it
    fn cache_op<Output, Action>(
        &self,
        counter_key: &str,
        event: &str,
        fault: Fault,
        op: Action,
    ) -> Result<Output, ReseedCacheError>
    where
        Action: FnOnce() -> Output,
    {
        self.begin_cache(format!("{event}:{counter_key}"), fault)?;
        Ok(op())
    }

    // the increment maps hold this pod's running totals, the way the memory tier would
    pub(crate) fn bump(&self, counter_key: &str, increment: Decimal) -> Decimal {
        let mut increments = self.increments.lock().expect("the fake increment lock");
        let base = increments
            .get(counter_key)
            .copied()
            .unwrap_or(Decimal::ZERO);
        let next = base.checked_add(increment).unwrap_or(Decimal::MAX);
        increments.insert(counter_key.to_owned(), next);
        next
    }

    fn read_redis(&self, key: &str) -> CacheResult<Option<ReseedCacheValue>> {
        self.begin_cache(format!("redis_get:{key}"), Fault::RedisGet)?;
        Ok(self.redis_value())
    }

    fn store_set_nx(&self, key: &str, value: Decimal) -> CacheResult<Option<bool>> {
        self.journal.push(format!("redis_set_nx:{key}"));
        self.put_redis(ReseedCacheValue::Number(value));
        Ok(Some(true))
    }

    fn read_memory(&self, key: &str) -> CacheResult<Option<ReseedCacheValue>> {
        self.begin_cache(format!("memory_get:{key}"), Fault::MemoryGet)?;
        Ok(self.memory_value(key))
    }

    fn store_memory(&self, key: &str, value: Decimal) -> CacheResult<()> {
        self.begin_cache(format!("memory_set:{key}"), Fault::MemorySet)?;
        self.put_memory(key, ReseedCacheValue::Number(value));
        Ok(())
    }

    fn bump_local(&self, key: &str, increment: Decimal) -> CacheResult<Option<Decimal>> {
        let counted = self.cache_op(key, "increment_local", Fault::IncrementLocal, || {
            self.bump(key, increment)
        });
        Ok(Some(counted))
    }
}

impl ReseedCache for FakeLayers {
    type Error = ReseedCacheError;

    fn redis_configured(&self) -> bool {
        self.redis_configured
    }

    fn redis_get(
        &self,
        counter_key: &str,
    ) -> impl Future<Output = Result<Option<ReseedCacheValue>, ReseedCacheError>> + Send {
        ready(self.read_redis(counter_key))
    }

    fn redis_set_nx(
        &self,
        counter_key: &str,
        value: Decimal,
    ) -> impl Future<Output = Result<Option<bool>, ReseedCacheError>> + Send {
        ready(self.store_set_nx(counter_key, value))
    }

    fn memory_get(
        &self,
        counter_key: &str,
    ) -> impl Future<Output = Result<Option<ReseedCacheValue>, ReseedCacheError>> + Send {
        ready(self.read_memory(counter_key))
    }

    fn memory_set(
        &self,
        counter_key: &str,
        value: Decimal,
    ) -> impl Future<Output = Result<(), ReseedCacheError>> + Send {
        ready(self.store_memory(counter_key, value))
    }

    fn increment_local(
        &self,
        counter_key: &str,
        increment: Decimal,
    ) -> impl Future<Output = Result<Option<Decimal>, ReseedCacheError>> + Send {
        ready(self.bump_local(counter_key, increment))
    }
}

impl WindowReseedCache for FakeLayers {
    fn redis_increment(
        &self,
        counter_key: &str,
        amount: Decimal,
    ) -> impl Future<Output = Result<Decimal, ReseedCacheError>> + Send {
        self.journal.push(format!("redis_increment:{counter_key}"));
        let held = self.redis.lock().expect("the fake redis lock").clone();
        let base = match held {
            Some(ReseedCacheValue::Number(value)) => value,
            Some(_) | None => Decimal::ZERO,
        };
        let next = base.checked_add(amount).unwrap_or(Decimal::MAX);
        self.put_redis(ReseedCacheValue::Number(next));
        ready(Ok(next))
    }
}

#[async_trait]
impl CounterLayers for FakeLayers {
    async fn increment_redis(
        &self,
        counter_key: &str,
        increment: Decimal,
    ) -> Result<Decimal, LayersError> {
        self.journal.push(format!("increment_redis:{counter_key}"));
        if let Some(error) = self.fatal(Fault::IncrementRedis) {
            return Err(error);
        }
        Ok(self.bump(counter_key, increment))
    }

    async fn refresh_redis_ttl(&self, counter_key: &str) -> Result<bool, LayersError> {
        self.begin(format!("refresh_ttl:{counter_key}"), Fault::RefreshTtl)?;
        Ok(*self.refresh_answer.lock().expect("the fake refresh lock"))
    }

    async fn delete_redis(&self, counter_key: &str) -> Result<(), LayersError> {
        self.begin(format!("delete_redis:{counter_key}"), Fault::DeleteRedis)?;
        *self.redis.lock().expect("the fake redis lock") = None;
        Ok(())
    }

    async fn delete_memory(&self, counter_key: &str) -> Result<(), LayersError> {
        self.begin(format!("delete_memory:{counter_key}"), Fault::DeleteMemory)?;
        self.memory
            .lock()
            .expect("the fake memory lock")
            .remove(counter_key);
        Ok(())
    }

    async fn set_memory_value(
        &self,
        counter_key: &str,
        value: ReseedCacheValue,
    ) -> Result<(), LayersError> {
        self.begin(
            format!("set_memory_value:{counter_key}"),
            Fault::SetMemoryValue,
        )?;
        if value != ReseedCacheValue::Unreadable {
            self.put_memory(counter_key, value);
        }
        Ok(())
    }

    async fn increment_redis_pipeline(
        &self,
        pending: &[PendingSpendIncrement],
    ) -> Result<Vec<Decimal>, LayersError> {
        self.begin(format!("pipeline:{}", pending.len()), Fault::Pipeline)?;
        let scripted = self
            .pipeline_answer
            .lock()
            .expect("the fake pipeline lock")
            .clone();
        if let Some(amounts) = scripted {
            return Ok(amounts);
        }
        let mut amounts = Vec::with_capacity(pending.len());
        for item in pending {
            amounts.push(self.bump(&item.counter_key, item.increment));
        }
        Ok(amounts)
    }

    fn post_call(&self) -> Option<PostCallBatch> {
        if !self.redis_configured {
            return None;
        }
        self.journal.push("post_call".to_owned());
        let held = self
            .post_call_batch
            .lock()
            .expect("the fake post-call lock")
            .clone();
        let batch = held?;
        let ttl = batch.ttl;
        let written: Arc<dyn CounterWriteBatch> = batch;
        Some(PostCallBatch {
            batch: written,
            ttl,
        })
    }
}

pub(crate) struct FakeBatch {
    pub(crate) answer: Mutex<Option<CounterValue>>,
    pub(crate) fails: AtomicBool,
    pub(crate) journal: Journal,
    pub(crate) recorded: Mutex<Vec<(String, Decimal)>>,
}

impl FakeBatch {
    pub(crate) fn new(journal: Journal) -> Self {
        Self {
            answer: Mutex::new(None),
            fails: AtomicBool::new(false),
            journal,
            recorded: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn answer_with(&self, value: CounterValue) {
        *self.answer.lock().expect("the fake batch answer lock") = Some(value);
    }

    pub(crate) fn fail(&self) {
        self.fails.store(true, Ordering::Relaxed);
    }

    pub(crate) fn recorded(&self) -> Vec<(String, Decimal)> {
        self.recorded
            .lock()
            .expect("the fake batch record lock")
            .clone()
    }
}

#[async_trait]
impl CounterBatch for FakeBatch {
    async fn read(
        &self,
        counter_key: &str,
    ) -> Result<Option<CounterValue>, SpendCounterBatchError> {
        self.journal.push(format!("batch_read:{counter_key}"));
        if self.fails.load(Ordering::Relaxed) {
            return Err(SpendCounterBatchError::Cancelled);
        }
        Ok(*self.answer.lock().expect("the fake batch answer lock"))
    }

    fn record(&self, counter_key: &str, value: Decimal) {
        self.journal
            .push(format!("batch_record:{counter_key}:{value}"));
        self.recorded
            .lock()
            .expect("the fake batch record lock")
            .push((counter_key.to_owned(), value));
    }

    fn forget(&self, counter_key: &str) {
        self.journal.push(format!("batch_forget:{counter_key}"));
    }
}

pub(crate) struct Queued {
    pub(crate) counter_key: String,
    pub(crate) increment: f64,
    pub(crate) ttl: i64,
    pub(crate) hook: SettledHook<f64>,
}

pub(crate) struct FakePostCall {
    pub(crate) ttl: i64,
    pub(crate) queued: Mutex<Vec<Queued>>,
    pub(crate) journal: Journal,
}

impl FakePostCall {
    pub(crate) fn new(ttl: i64, journal: Journal) -> Self {
        Self {
            ttl,
            queued: Mutex::new(Vec::new()),
            journal,
        }
    }

    pub(crate) fn queued(&self) -> Vec<(String, f64, i64)> {
        self.queued
            .lock()
            .expect("the fake post-call lock")
            .iter()
            .map(|item| (item.counter_key.clone(), item.increment, item.ttl))
            .collect()
    }

    pub(crate) async fn fire(&self, index: usize, outcome: Settled<f64>) {
        let hook = {
            let queued = self.queued.lock().expect("the fake post-call lock");
            Arc::clone(&queued[index].hook)
        };
        if let Some(follow) = hook(&outcome) {
            follow.await;
        }
    }
}

impl CounterWriteBatch for FakePostCall {
    fn increment(&self, counter_key: &str, increment: f64, ttl: i64, hook: SettledHook<f64>) {
        self.journal.push(format!("queue:{counter_key}"));
        self.queued
            .lock()
            .expect("the fake post-call lock")
            .push(Queued {
                counter_key: counter_key.to_owned(),
                increment,
                ttl,
                hook,
            });
    }
}

pub(crate) struct FakeSources {
    pub(crate) values: Mutex<BTreeMap<String, Json>>,
}

impl FakeSources {
    pub(crate) fn empty() -> Self {
        Self {
            values: Mutex::new(BTreeMap::new()),
        }
    }

    pub(crate) fn put(&self, cache_key: &str, value: Json) {
        self.values
            .lock()
            .expect("the fake source lock")
            .insert(cache_key.to_owned(), value);
    }
}

#[async_trait]
impl SourceSpendCache for FakeSources {
    async fn cached(&self, cache_key: &str) -> Option<Json> {
        let values = self.values.lock().expect("the fake source lock");
        values.get(cache_key).cloned()
    }
}

pub(crate) struct FixedClock {
    pub(crate) now: DateTime<Utc>,
}

impl BudgetClock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.now
    }

    fn uptime(&self) -> Duration {
        Duration::ZERO
    }
}

pub(crate) struct FakeDurations;

impl DurationParser for FakeDurations {
    fn seconds(&self, spec: &str, _now: DateTime<Utc>) -> Option<i64> {
        match spec {
            "1h" => Some(3_600),
            "1d" => Some(86_400),
            _ => None,
        }
    }
}

pub(crate) fn fixture_instant() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 18, 12, 0, 0)
        .single()
        .expect("a fixture instant")
}

fn reseed_of(rows: Arc<FakeRows>) -> SpendCounterReseed<FakeRows> {
    let clock = FixedClock {
        now: fixture_instant(),
    };
    let gate = LoopBoundSemaphore {
        value: 25,
        slot: Mutex::new(None),
    };
    let stall_tracker = DbLookupStallTracker {
        clock: SystemClock,
        last_hit: Mutex::new(None),
    };
    SpendCounterReseed {
        rows: Some(rows),
        locks: LockRegistry::from(10_000),
        clock: Box::new(clock),
        durations: Arc::new(FakeDurations),
        tasks: TaskTracker::new(),
        gate: Arc::new(gate),
        stall_tracker: Arc::new(stall_tracker),
        deadline_seconds: 5.0,
    }
}

pub(crate) struct Fixture {
    pub(crate) layers: Arc<FakeLayers>,
    pub(crate) batch: Arc<FakeBatch>,
    pub(crate) rows: Arc<FakeRows>,
    pub(crate) sources: Arc<FakeSources>,
    pub(crate) journal: Journal,
    pub(crate) writes: SpendCounterWrites<FakeLayers, FakeRows>,
}

pub(crate) fn fixture(redis_configured: bool) -> Fixture {
    let journal = Journal::default();
    let layers = Arc::new(FakeLayers::new(redis_configured, journal.clone()));
    let batch = Arc::new(FakeBatch::new(journal.clone()));
    let rows = Arc::new(FakeRows::cold());
    let sources = Arc::new(FakeSources::empty());
    let reseed = Arc::new(reseed_of(Arc::clone(&rows)));
    let batch_handle = Arc::clone(&batch);
    let counted: Arc<dyn CounterBatch> = batch_handle;
    let source_handle = Arc::clone(&sources);
    let source_tier: Arc<dyn SourceSpendCache> = source_handle;
    let writes = SpendCounterWrites {
        layers: Arc::clone(&layers),
        batch: counted,
        reseed,
        sources: source_tier,
    };
    Fixture {
        layers,
        batch,
        rows,
        sources,
        journal,
        writes,
    }
}

pub(crate) fn local_memory() -> Arc<LocalMemoryOps> {
    let clock: Arc<dyn Fn() -> f64 + Send + Sync> = Arc::new(|| 1_000.0_f64);
    let memory = LocalMemoryOps::new(1024, clock).expect("the local memory tier builds");
    Arc::new(memory)
}

fn dual_settings() -> DualCacheSettings {
    DualCacheSettings {
        default_in_memory_ttl: None,
        default_redis_ttl: None,
        batch_cache_expiry: None,
        max_redis_batch_cache_size: 1_000,
        redis_delete_chunk_size: 1_000,
    }
}

pub(crate) fn dual_cache(redis: Option<Arc<RedisCache>>) -> DualCache<LocalMemoryOps> {
    DualCache::new(local_memory(), redis, dual_settings())
}

pub(crate) fn auth_cache() -> AuthCache<DualCache<LocalMemoryOps>> {
    gateway_cache(local_memory(), local_memory(), None, dual_settings(), 60.0)
}

pub(crate) fn writes_over<Layers>(
    layers: Arc<Layers>,
    batch: Arc<FakeBatch>,
) -> SpendCounterWrites<Layers, FakeRows>
where
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers,
{
    let counted: Arc<dyn CounterBatch> = batch;
    let sources: Arc<dyn SourceSpendCache> = Arc::new(FakeSources::empty());
    let rows = Arc::new(FakeRows::cold());
    SpendCounterWrites {
        layers,
        batch: counted,
        reseed: Arc::new(reseed_of(rows)),
        sources,
    }
}

#[derive(Default)]
struct LogFields(Option<String>);

impl Visit for LogFields {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0 = Some(value.to_owned());
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = Some(format!("{value:?}"));
        }
    }
}

struct Recorded {
    level: Level,
    message: String,
}

#[derive(Clone, Default)]
struct MessageLog(Arc<Mutex<Vec<Recorded>>>);

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for MessageLog {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        let mut fields = LogFields::default();
        event.record(&mut fields);
        if let Some(message) = fields.0
            && let Ok(mut messages) = self.0.lock()
        {
            messages.push(Recorded {
                level: *event.metadata().level(),
                message,
            });
        }
    }
}

pub(crate) struct Logs {
    messages: MessageLog,
    _guard: DefaultGuard,
}

impl Logs {
    pub(crate) fn capture() -> Self {
        let messages = MessageLog::default();
        let subscriber = tracing_subscriber::registry().with(messages.clone());
        Self {
            messages,
            _guard: set_default(subscriber),
        }
    }

    pub(crate) fn contains(&self, text: &str) -> bool {
        self.messages
            .0
            .lock()
            .is_ok_and(|messages| messages.iter().any(|entry| entry.message.contains(text)))
    }

    pub(crate) fn warnings(&self) -> Vec<String> {
        let messages = self.messages.0.lock().expect("the fake log lock");
        messages
            .iter()
            .filter(|entry| entry.level == Level::WARN)
            .map(|entry| entry.message.clone())
            .collect()
    }
}
