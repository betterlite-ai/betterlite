use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::database_failures::gate::{
    DbLookupStallTracker, LoopBoundSemaphore, SystemClock,
};
use betterlite_db::spend_rows::rows::{
    SpendCounterRows as DbSpendCounterRows, SpendRow as DbSpendRow, SpendRowLookup,
};
use betterlite_db::window_queue::{WindowQueueError, WindowSpendTransaction};
use betterlite_models::decisions::Json;
use chrono::{DateTime, TimeZone as _, Utc};
use rust_decimal::Decimal;
use tokio_util::task::TaskTracker;
use tracing::Event;
use tracing::field::{Field, Visit};
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

use crate::budget_counter::CounterEntityType;
use crate::budget_primitives::window_start::DurationParser;
use crate::model_budget_limiter::clock::BudgetClock;
use crate::reservation_settlement::counters::{ScopeHold, SettlementCounters};
use crate::reservation_settlement::entry::ReservationEntry;
use crate::reservation_settlement::error::{CounterReadError, SettlementError};
use crate::reservation_settlement::handle::ReservationHandle;
use crate::spend_counter_batch::batch::{
    CounterValue, PendingSpendIncrement, SpendCounterBatchError,
};
use crate::spend_counter_batch::scope::SpendCounterScope;
use crate::spend_increments::window::{WindowQueueFuture, WindowSpendQueue};
use crate::spend_increments::{SpendIncrementInput, SpendIncrementServices};
use crate::spend_reseed::cache::dual::ReseedCacheError;
use crate::spend_reseed::cache::{ReseedCache, WindowReseedCache};
use crate::spend_reseed::locks::LockRegistry;
use crate::spend_reseed::reseed::SpendCounterReseed;
use crate::spend_reseed::rows::{
    SpendAggregateRow, SpendCounterRows, WindowEntity, WindowSpendRow,
};
use crate::spend_reseed::values::ReseedCacheValue;
use crate::spend_writes::SpendCounterWrites;
use crate::spend_writes::batch::CounterBatch;
use crate::spend_writes::error::LayersError;
use crate::spend_writes::layers::{CounterLayers, PostCallBatch};
use crate::spend_writes::source::SourceSpendCache;

pub(crate) fn decimal(text: &str) -> Decimal {
    text.parse().expect("a decimal amount")
}

pub(crate) fn fixture_instant() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 18, 12, 0, 0)
        .single()
        .expect("a fixture instant")
}

pub(crate) fn texts(values: &[&str]) -> Vec<JsonData> {
    values
        .iter()
        .map(|text| JsonData::String((*text).to_owned()))
        .collect()
}

pub(crate) fn input(token: Option<&str>) -> SpendIncrementInput<'_> {
    SpendIncrementInput {
        token,
        team_id: None,
        user_id: None,
        response_cost: Some(decimal("2")),
        org_id: None,
        budget_reservation: None,
        end_user_id: None,
        tags: None,
        request_started_at: None,
        model_access_groups: None,
        project_id: None,
    }
}

// the cache a scope writes through: a local memory tier with numbers, and a Redis tier that answers a clean miss so a spy sees the pipeline
pub(crate) struct SpyLayers {
    redis: bool,
    memory: Mutex<BTreeMap<String, Decimal>>,
    pipelines: Mutex<Vec<Vec<PendingSpendIncrement>>>,
}

impl SpyLayers {
    fn new(redis: bool) -> Self {
        Self {
            redis,
            memory: Mutex::new(BTreeMap::new()),
            pipelines: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn hold(&self, counter_key: &str, amount: Decimal) {
        self.memory
            .lock()
            .expect("the fake cache lock")
            .insert(counter_key.to_owned(), amount);
    }

    pub(crate) fn pipelines(&self) -> Vec<Vec<PendingSpendIncrement>> {
        self.pipelines
            .lock()
            .expect("the fake pipeline lock")
            .clone()
    }

    fn increment_memory(
        &self,
        counter_key: &str,
        increment: Decimal,
    ) -> Result<Decimal, ReseedCacheError> {
        let mut memory = self.memory.lock().expect("the fake cache lock");
        let current = memory.get(counter_key).copied().unwrap_or(Decimal::ZERO);
        let next = current
            .checked_add(increment)
            .ok_or(ReseedCacheError::OutOfRange(increment))?;
        memory.insert(counter_key.to_owned(), next);
        Ok(next)
    }
}

impl ReseedCache for SpyLayers {
    type Error = ReseedCacheError;

    fn redis_configured(&self) -> bool {
        self.redis
    }

    fn redis_get(
        &self,
        _counter_key: &str,
    ) -> impl Future<Output = Result<Option<ReseedCacheValue>, ReseedCacheError>> + Send {
        ready(Ok(None))
    }

    fn redis_set_nx(
        &self,
        _counter_key: &str,
        _value: Decimal,
    ) -> impl Future<Output = Result<Option<bool>, ReseedCacheError>> + Send {
        ready(Ok(Some(true)))
    }

    fn memory_get(
        &self,
        counter_key: &str,
    ) -> impl Future<Output = Result<Option<ReseedCacheValue>, ReseedCacheError>> + Send {
        let value = self
            .memory
            .lock()
            .expect("the fake cache lock")
            .get(counter_key)
            .copied();
        ready(Ok(value.map(ReseedCacheValue::Number)))
    }

    fn memory_set(
        &self,
        counter_key: &str,
        value: Decimal,
    ) -> impl Future<Output = Result<(), ReseedCacheError>> + Send {
        self.hold(counter_key, value);
        ready(Ok(()))
    }

    fn increment_local(
        &self,
        counter_key: &str,
        increment: Decimal,
    ) -> impl Future<Output = Result<Option<Decimal>, ReseedCacheError>> + Send {
        let counted = self.increment_memory(counter_key, increment);
        ready(counted.map(Some))
    }
}

impl WindowReseedCache for SpyLayers {
    fn redis_increment(
        &self,
        counter_key: &str,
        amount: Decimal,
    ) -> impl Future<Output = Result<Decimal, ReseedCacheError>> + Send {
        let counted = self.increment_memory(counter_key, amount);
        ready(counted)
    }
}

#[async_trait]
impl CounterLayers for SpyLayers {
    async fn increment_redis(
        &self,
        counter_key: &str,
        increment: Decimal,
    ) -> Result<Decimal, LayersError> {
        self.increment_memory(counter_key, increment)
            .map_err(|_error| LayersError::Unavailable)
    }

    async fn refresh_redis_ttl(&self, _counter_key: &str) -> Result<bool, LayersError> {
        Ok(true)
    }

    async fn delete_redis(&self, _counter_key: &str) -> Result<(), LayersError> {
        Ok(())
    }

    async fn delete_memory(&self, counter_key: &str) -> Result<(), LayersError> {
        self.memory
            .lock()
            .expect("the fake cache lock")
            .remove(counter_key);
        Ok(())
    }

    async fn set_memory_value(
        &self,
        counter_key: &str,
        value: ReseedCacheValue,
    ) -> Result<(), LayersError> {
        if let ReseedCacheValue::Number(amount) = value {
            self.hold(counter_key, amount);
        }
        Ok(())
    }

    async fn increment_redis_pipeline(
        &self,
        pending: &[PendingSpendIncrement],
    ) -> Result<Vec<Decimal>, LayersError> {
        self.pipelines
            .lock()
            .expect("the fake pipeline lock")
            .push(pending.to_vec());
        let mut amounts = Vec::with_capacity(pending.len());
        for item in pending {
            amounts.push(
                self.increment_memory(&item.counter_key, item.increment)
                    .map_err(|_error| LayersError::Unavailable)?,
            );
        }
        Ok(amounts)
    }

    fn post_call(&self) -> Option<PostCallBatch> {
        None
    }
}

#[derive(Default)]
pub(crate) struct FakeBatch {
    failing: Mutex<BTreeSet<String>>,
}

impl FakeBatch {
    pub(crate) fn fail_for(&self, counter_key: &str) {
        self.failing
            .lock()
            .expect("the fake batch lock")
            .insert(counter_key.to_owned());
    }
}

#[async_trait]
impl CounterBatch for FakeBatch {
    async fn read(
        &self,
        counter_key: &str,
    ) -> Result<Option<CounterValue>, SpendCounterBatchError> {
        if self
            .failing
            .lock()
            .expect("the fake batch lock")
            .contains(counter_key)
        {
            return Err(SpendCounterBatchError::Cancelled);
        }
        Ok(None)
    }

    fn record(&self, _counter_key: &str, _value: Decimal) {}

    fn forget(&self, _counter_key: &str) {}
}

#[derive(Default)]
pub(crate) struct FakeSources {
    values: Mutex<BTreeMap<String, Json>>,
    reads: Mutex<Vec<String>>,
}

impl FakeSources {
    pub(crate) fn put(&self, cache_key: &str, text: &str) {
        let value: Json = serde_json::from_str(text).expect("a source cache value");
        self.values
            .lock()
            .expect("the fake source lock")
            .insert(cache_key.to_owned(), value);
    }

    pub(crate) fn reads(&self) -> Vec<String> {
        self.reads.lock().expect("the fake source lock").clone()
    }
}

#[async_trait]
impl SourceSpendCache for FakeSources {
    async fn cached(&self, cache_key: &str) -> Option<Json> {
        self.reads
            .lock()
            .expect("the fake source lock")
            .push(cache_key.to_owned());
        self.values
            .lock()
            .expect("the fake source lock")
            .get(cache_key)
            .cloned()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the fake row store failed")]
pub(crate) struct FakeError;

#[derive(Default)]
pub(crate) struct FakeRows {
    failing: AtomicBool,
}

impl FakeRows {
    pub(crate) fn fail(&self) {
        self.failing.store(true, Ordering::Relaxed);
    }
}

impl DbSpendCounterRows for FakeRows {
    type Error = FakeError;

    fn spend_row(
        &self,
        _lookup: SpendRowLookup<'_>,
    ) -> impl Future<Output = Result<Option<DbSpendRow>, FakeError>> + Send {
        if self.failing.load(Ordering::Relaxed) {
            return ready(Err(FakeError));
        }
        ready(Ok(None))
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
        if self.failing.load(Ordering::Relaxed) {
            return ready(Err(FakeError));
        }
        ready(Ok(None))
    }

    fn window_log_rows(
        &self,
        _entity: WindowEntity,
        _entity_id: &str,
        _window_start: DateTime<Utc>,
    ) -> impl Future<Output = Result<Vec<SpendAggregateRow>, FakeError>> + Send {
        if self.failing.load(Ordering::Relaxed) {
            return ready(Err(FakeError));
        }
        ready(Ok(Vec::new()))
    }
}

#[derive(Default)]
pub(crate) struct FakeWindowQueue {
    transactions: Mutex<Vec<WindowSpendTransaction>>,
    failing: AtomicBool,
}

impl FakeWindowQueue {
    pub(crate) fn fail(&self) {
        self.failing.store(true, Ordering::Relaxed);
    }

    pub(crate) fn records(&self) -> Vec<WindowSpendTransaction> {
        self.transactions
            .lock()
            .expect("the fake window queue lock")
            .clone()
    }
}

impl WindowSpendQueue for FakeWindowQueue {
    fn enqueue(&self, transaction: WindowSpendTransaction) -> WindowQueueFuture<'_> {
        if self.failing.load(Ordering::Relaxed) {
            return Box::pin(ready(Err(WindowQueueError::ZeroCapacity)));
        }
        self.transactions
            .lock()
            .expect("the fake window queue lock")
            .push(transaction);
        Box::pin(ready(Ok(())))
    }
}

pub(crate) struct NoHold;

impl ScopeHold for NoHold {}

// the settlement counters B3003 reaches, over the same memory the scope writes so a reserved counter settles in place
pub(crate) struct FakeCounters {
    layers: Arc<SpyLayers>,
    reseeds: AtomicBool,
    fail_increment: AtomicBool,
    invalidated: Mutex<Vec<String>>,
    increments: Mutex<Vec<(String, Decimal)>>,
}

impl FakeCounters {
    fn new(layers: Arc<SpyLayers>) -> Self {
        Self {
            layers,
            reseeds: AtomicBool::new(true),
            fail_increment: AtomicBool::new(false),
            invalidated: Mutex::new(Vec::new()),
            increments: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn reseed_unavailable(&self) {
        self.reseeds.store(false, Ordering::Relaxed);
    }

    pub(crate) fn fail_increment(&self) {
        self.fail_increment.store(true, Ordering::Relaxed);
    }

    pub(crate) fn invalidated(&self) -> Vec<String> {
        self.invalidated
            .lock()
            .expect("the fake settlement lock")
            .clone()
    }

    pub(crate) fn increments(&self) -> Vec<(String, Decimal)> {
        self.increments
            .lock()
            .expect("the fake settlement lock")
            .clone()
    }
}

#[async_trait]
impl SettlementCounters for FakeCounters {
    fn enter_counters_batch_scope(
        &self,
        _counter_keys: &BTreeSet<String>,
    ) -> Box<dyn ScopeHold + Send + '_> {
        Box::new(NoHold)
    }

    async fn read_cache_value(
        &self,
        counter_key: &str,
    ) -> Result<Option<Decimal>, CounterReadError> {
        match ReseedCache::memory_get(&*self.layers, counter_key).await {
            Ok(Some(ReseedCacheValue::Number(amount))) => Ok(Some(amount)),
            Ok(Some(
                ReseedCacheValue::Text(_)
                | ReseedCacheValue::Boolean(_)
                | ReseedCacheValue::Unreadable,
            ))
            | Ok(None) => Ok(None),
            Err(_error) => Err(CounterReadError::NotNumeric {
                counter_key: counter_key.to_owned(),
            }),
        }
    }

    async fn reseed_from_db(&self, _counter_key: &str) -> bool {
        self.reseeds.load(Ordering::Relaxed)
    }

    async fn increment_counter(
        &self,
        counter_key: &str,
        increment: Decimal,
    ) -> Result<(), SettlementError> {
        if self.fail_increment.load(Ordering::Relaxed) {
            return Err(SettlementError::Overflow);
        }
        self.increments
            .lock()
            .expect("the fake settlement lock")
            .push((counter_key.to_owned(), increment));
        self.layers
            .increment_memory(counter_key, increment)
            .map_err(|_error| SettlementError::Overflow)?;
        Ok(())
    }

    async fn increment_pipeline(
        &self,
        pending: &[PendingSpendIncrement],
    ) -> Result<(), SettlementError> {
        for item in pending {
            self.increment_counter(&item.counter_key, item.increment)
                .await?;
        }
        Ok(())
    }

    async fn invalidate(&self, counter_key: &str) {
        self.invalidated
            .lock()
            .expect("the fake settlement lock")
            .push(counter_key.to_owned());
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

fn reseed_of(rows: Arc<FakeRows>) -> SpendCounterReseed<FakeRows> {
    SpendCounterReseed {
        rows: Some(rows),
        locks: LockRegistry::from(10_000),
        clock: Box::new(FixedClock {
            now: fixture_instant(),
        }),
        durations: Arc::new(FakeDurations),
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

pub(crate) struct Fixture {
    pub(crate) batch: Arc<FakeBatch>,
    pub(crate) rows: Arc<FakeRows>,
    pub(crate) sources: Arc<FakeSources>,
    pub(crate) queue: Arc<FakeWindowQueue>,
    pub(crate) counters: FakeCounters,
    pub(crate) scope: SpendCounterScope,
    pub(crate) layers: Arc<SpyLayers>,
    pub(crate) writes: SpendCounterWrites<SpyLayers, FakeRows>,
}

impl Fixture {
    pub(crate) fn new(redis: bool) -> Self {
        let batch = Arc::new(FakeBatch::default());
        let rows = Arc::new(FakeRows::default());
        let sources = Arc::new(FakeSources::default());
        let layers = Arc::new(SpyLayers::new(redis));
        let counted: Arc<dyn CounterBatch> = Arc::clone(&batch);
        let source_tier: Arc<dyn SourceSpendCache> = Arc::clone(&sources);
        Self {
            writes: SpendCounterWrites {
                layers: Arc::clone(&layers),
                batch: counted,
                reseed: Arc::new(reseed_of(Arc::clone(&rows))),
                sources: source_tier,
            },
            counters: FakeCounters::new(Arc::clone(&layers)),
            batch,
            rows,
            sources,
            queue: Arc::new(FakeWindowQueue::default()),
            scope: SpendCounterScope::default(),
            layers,
        }
    }

    pub(crate) fn services(&self) -> SpendIncrementServices<'_, FakeCounters, SpyLayers, FakeRows> {
        SpendIncrementServices {
            writes: &self.writes,
            scope: &self.scope,
            redis: None,
            window_queue: self.queue.as_ref(),
            counters: &self.counters,
        }
    }

    pub(crate) async fn counter(&self, counter_key: &str) -> Option<Decimal> {
        match ReseedCache::memory_get(&*self.layers, counter_key).await {
            Ok(Some(ReseedCacheValue::Number(amount))) => Some(amount),
            Ok(Some(
                ReseedCacheValue::Text(_)
                | ReseedCacheValue::Boolean(_)
                | ReseedCacheValue::Unreadable,
            ))
            | Ok(None) => None,
            Err(error) => panic!("the fake cache read failed: {error}"),
        }
    }
}

pub(crate) fn handle(reserved_cost: &str, counter_keys: &[&str]) -> ReservationHandle {
    let entries = counter_keys
        .iter()
        .map(|counter_key| ReservationEntry {
            counter_key: Some((*counter_key).to_owned()),
            entity_type: CounterEntityType::User,
            entity_id: "u1".to_owned(),
            reserved_cost: Some(decimal(reserved_cost)),
            applied_adjustment: Decimal::ZERO,
        })
        .collect();
    ReservationHandle::new(decimal(reserved_cost), entries, Decimal::ZERO)
}

pub(crate) fn finalized(handle: &ReservationHandle) -> bool {
    betterlite_redis::request_batch::lock(&handle.0).finalized
}

pub(crate) fn applied_adjustment(handle: &ReservationHandle) -> Option<Decimal> {
    betterlite_redis::request_batch::lock(&handle.0)
        .entries
        .first()
        .map(|entry| entry.applied_adjustment)
}

#[derive(Default)]
struct LogFields(Option<String>);

impl Visit for LogFields {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0 = Some(value.to_owned());
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = Some(format!("{value:?}"));
        }
    }
}

#[derive(Clone, Default)]
struct MessageLog(Arc<Mutex<Vec<String>>>);

impl<S: tracing::Subscriber> Layer<S> for MessageLog {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        let mut fields = LogFields::default();
        event.record(&mut fields);
        if let Some(message) = fields.0
            && let Ok(mut messages) = self.0.lock()
        {
            messages.push(message);
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
            .is_ok_and(|messages| messages.iter().any(|message| message.contains(text)))
    }
}
