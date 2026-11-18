use std::collections::{BTreeMap, VecDeque};
use std::error::Error;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use betterlite_db::database_failures::gate::{
    DbLookupStallTracker, LoopBoundSemaphore, SystemClock,
};
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::redis_cache::connect::{RedisCacheOptions, connect};
use betterlite_redis::redis_client_reuse::breaker::{BreakerAdmission, BreakerOpen};
use betterlite_redis::redis_client_reuse::breaker_gate::BreakerGate;
use betterlite_redis::redis_client_reuse::service_event::ServiceEvent;
use betterlite_redis::redis_client_reuse::service_hooks::ServiceHooks;
use betterlite_redis::redis_client_reuse::store::RedisClientStore;
use betterlite_redis::redis_client_reuse::transport::RedisCacheFamily;
use betterlite_redis::redis_connection_settings::model::settings::RedisOverrides;
use betterlite_redis::redis_live_client::LiveClientFactory;
use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeZone as _, Utc};
use rust_decimal::Decimal;
use tokio::time::sleep;
use tokio_util::task::TaskTracker;

use crate::budget_primitives::window_start::DurationParser;
use crate::model_budget_limiter::clock::BudgetClock;
use crate::spend_counter_batch::batch::{CounterValue, SpendCounterBatchError};
use crate::spend_reseed::cache::{ActiveSpendBatch, ReseedCache};
use crate::spend_reseed::locks::LockRegistry;
use crate::spend_reseed::reseed::SpendCounterReseed;
use crate::spend_reseed::rows::{
    SpendAggregateRow, SpendCounterRows, SpendRow, SpendRowLookup, WindowEntity, WindowSpendRow,
};
use crate::spend_reseed::values::ReseedCacheValue;

#[derive(Debug, thiserror::Error)]
#[error("the fake store failed")]
pub(crate) struct FakeError;

pub(crate) fn decimal(text: &str) -> Decimal {
    text.parse().expect("a fixture decimal parses")
}

// a coalesced answer whose cache error type has no equality, unwrapped for an assert
pub(crate) fn answered<Failure>(outcome: Result<Option<Decimal>, Failure>) -> Option<Decimal>
where
    Failure: std::fmt::Debug,
{
    outcome.expect("the reseed answers")
}

pub(crate) fn spend_json(text: &str) -> JsonData {
    serde_json::from_str(text).expect("a fixture JSON value parses")
}

pub(crate) fn spend_row(text: &str) -> SpendRow {
    SpendRow {
        spend: Some(spend_json(text)),
    }
}

pub(crate) fn utc(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .expect("a fixture timestamp parses")
        .with_timezone(&Utc)
}

pub(crate) fn aware(text: &str) -> Timestamp {
    Timestamp::Aware(
        DateTime::<FixedOffset>::parse_from_rfc3339(text)
            .expect("a fixture offset timestamp parses"),
    )
}

pub(crate) fn naive(text: &str) -> Timestamp {
    Timestamp::Naive(
        NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S")
            .expect("a fixture naive timestamp parses"),
    )
}

pub(crate) fn window_spend_row(window_start: Timestamp, spend: &str) -> WindowSpendRow {
    WindowSpendRow {
        window_start,
        spend: Some(spend_json(spend)),
    }
}

pub(crate) fn mapping_sum(spend: &str) -> SpendAggregateRow {
    SpendAggregateRow::Mapping(BTreeMap::from([(
        "_sum".to_owned(),
        JsonData::Object(BTreeMap::from([("spend".to_owned(), spend_json(spend))])),
    )]))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum LookupRecord {
    Key(String),
    TeamMember { user_id: String, team_id: String },
    Team(String),
    User(String),
    Organization(String),
    Project(String),
    EndUser(String),
}

impl From<SpendRowLookup<'_>> for LookupRecord {
    fn from(lookup: SpendRowLookup<'_>) -> Self {
        match lookup {
            SpendRowLookup::Key(id) => Self::Key(id.to_owned()),
            SpendRowLookup::TeamMember { user_id, team_id } => Self::TeamMember {
                user_id: user_id.to_owned(),
                team_id: team_id.to_owned(),
            },
            SpendRowLookup::Team(id) => Self::Team(id.to_owned()),
            SpendRowLookup::User(id) => Self::User(id.to_owned()),
            SpendRowLookup::Organization(id) => Self::Organization(id.to_owned()),
            SpendRowLookup::Project(id) => Self::Project(id.to_owned()),
            SpendRowLookup::EndUser(id) => Self::EndUser(id.to_owned()),
        }
    }
}

pub(crate) struct FakeRows {
    pub(crate) row: Mutex<Option<SpendRow>>,
    pub(crate) window: Mutex<Option<WindowSpendRow>>,
    pub(crate) logs: Mutex<Vec<SpendAggregateRow>>,
    pub(crate) lookups: Mutex<Vec<LookupRecord>>,
    pub(crate) window_lookups: Mutex<Vec<(WindowEntity, String, String)>>,
    pub(crate) log_lookups: Mutex<Vec<(WindowEntity, String, DateTime<Utc>)>>,
    pub(crate) rows_fail: AtomicBool,
    pub(crate) window_fails: AtomicBool,
    pub(crate) logs_fail: AtomicBool,
    pub(crate) delay: Duration,
}

impl FakeRows {
    pub(crate) fn empty() -> Self {
        Self {
            row: Mutex::new(None),
            window: Mutex::new(None),
            logs: Mutex::new(Vec::new()),
            lookups: Mutex::new(Vec::new()),
            window_lookups: Mutex::new(Vec::new()),
            log_lookups: Mutex::new(Vec::new()),
            rows_fail: AtomicBool::new(false),
            window_fails: AtomicBool::new(false),
            logs_fail: AtomicBool::new(false),
            delay: Duration::ZERO,
        }
    }

    pub(crate) fn with_row(row: SpendRow) -> Self {
        let rows = Self::empty();
        *rows.row.lock().expect("the fake row lock") = Some(row);
        rows
    }

    pub(crate) fn lookups(&self) -> Vec<LookupRecord> {
        self.lookups.lock().expect("the fake lookup lock").clone()
    }

    pub(crate) fn window_lookups(&self) -> Vec<(WindowEntity, String, String)> {
        self.window_lookups
            .lock()
            .expect("the fake window lock")
            .clone()
    }

    pub(crate) fn log_lookups(&self) -> Vec<(WindowEntity, String, DateTime<Utc>)> {
        self.log_lookups.lock().expect("the fake log lock").clone()
    }
}

impl SpendCounterRows for FakeRows {
    type Error = FakeError;

    async fn spend_row(&self, lookup: SpendRowLookup<'_>) -> Result<Option<SpendRow>, FakeError> {
        self.lookups
            .lock()
            .expect("the fake lookup lock")
            .push(LookupRecord::from(lookup));
        if self.rows_fail.load(Ordering::Relaxed) {
            return Err(FakeError);
        }
        sleep(self.delay).await;
        Ok(self.row.lock().expect("the fake row lock").clone())
    }

    async fn window_row(
        &self,
        entity: WindowEntity,
        entity_id: &str,
        window_duration: &str,
    ) -> Result<Option<WindowSpendRow>, FakeError> {
        self.window_lookups
            .lock()
            .expect("the fake window lock")
            .push((entity, entity_id.to_owned(), window_duration.to_owned()));
        if self.window_fails.load(Ordering::Relaxed) {
            return Err(FakeError);
        }
        Ok(self
            .window
            .lock()
            .expect("the fake window row lock")
            .clone())
    }

    async fn window_log_rows(
        &self,
        entity: WindowEntity,
        entity_id: &str,
        window_start: DateTime<Utc>,
    ) -> Result<Vec<SpendAggregateRow>, FakeError> {
        self.log_lookups.lock().expect("the fake log lock").push((
            entity,
            entity_id.to_owned(),
            window_start,
        ));
        if self.logs_fail.load(Ordering::Relaxed) {
            return Err(FakeError);
        }
        Ok(self.logs.lock().expect("the fake logs lock").clone())
    }
}

#[derive(Clone, Debug)]
pub(crate) enum RedisAnswer {
    Hit(ReseedCacheValue),
    Miss,
    Fail,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum SetAnswer {
    Stored,
    Taken,
    Fail,
}

pub(crate) struct FakeCache {
    pub(crate) redis_configured: bool,
    pub(crate) redis: Mutex<Option<ReseedCacheValue>>,
    pub(crate) redis_answers: Mutex<VecDeque<RedisAnswer>>,
    pub(crate) memory_answers: Mutex<VecDeque<Option<ReseedCacheValue>>>,
    pub(crate) set_answer: Mutex<SetAnswer>,
    pub(crate) memory: Mutex<BTreeMap<String, ReseedCacheValue>>,
    pub(crate) memory_sets: Mutex<Vec<(String, Decimal)>>,
    pub(crate) redis_sets: Mutex<Vec<(String, Decimal)>>,
    pub(crate) increments: Mutex<BTreeMap<String, Decimal>>,
    pub(crate) redis_gets: AtomicUsize,
    pub(crate) memory_gets: AtomicUsize,
    pub(crate) memory_get_fails: AtomicBool,
    pub(crate) memory_set_fails: AtomicBool,
    pub(crate) increment_fails: AtomicBool,
    pub(crate) increment_delay: Duration,
}

impl FakeCache {
    pub(crate) fn new(redis_configured: bool) -> Self {
        Self {
            redis_configured,
            redis: Mutex::new(None),
            redis_answers: Mutex::new(VecDeque::new()),
            memory_answers: Mutex::new(VecDeque::new()),
            set_answer: Mutex::new(SetAnswer::Stored),
            memory: Mutex::new(BTreeMap::new()),
            memory_sets: Mutex::new(Vec::new()),
            redis_sets: Mutex::new(Vec::new()),
            increments: Mutex::new(BTreeMap::new()),
            redis_gets: AtomicUsize::new(0),
            memory_gets: AtomicUsize::new(0),
            memory_get_fails: AtomicBool::new(false),
            memory_set_fails: AtomicBool::new(false),
            increment_fails: AtomicBool::new(false),
            increment_delay: Duration::ZERO,
        }
    }

    pub(crate) fn with_memory(mut self, counter_key: &str, value: ReseedCacheValue) -> Self {
        self.memory
            .lock()
            .expect("the fake memory lock")
            .insert(counter_key.to_owned(), value);
        self
    }

    pub(crate) fn with_memory_answers<I>(mut self, answers: I) -> Self
    where
        I: IntoIterator<Item = Option<ReseedCacheValue>>,
    {
        *self.memory_answers.lock().expect("the fake answer lock") = answers.into_iter().collect();
        self
    }

    pub(crate) fn with_redis(mut self, value: ReseedCacheValue) -> Self {
        *self.redis.lock().expect("the fake redis lock") = Some(value);
        self
    }

    pub(crate) fn answer(mut self, answer: RedisAnswer) -> Self {
        self.redis_answers
            .lock()
            .expect("the fake answer lock")
            .push_back(answer);
        self
    }

    pub(crate) fn with_set_answer(mut self, answer: SetAnswer) -> Self {
        self.set_answer = Mutex::new(answer);
        self
    }

    pub(crate) fn memory_text(&self, counter_key: &str) -> Option<ReseedCacheValue> {
        self.memory
            .lock()
            .expect("the fake memory lock")
            .get(counter_key)
            .cloned()
    }

    pub(crate) fn memory_sets(&self) -> Vec<(String, Decimal)> {
        self.memory_sets
            .lock()
            .expect("the fake memory set lock")
            .clone()
    }

    pub(crate) fn redis_sets(&self) -> Vec<(String, Decimal)> {
        self.redis_sets
            .lock()
            .expect("the fake redis set lock")
            .clone()
    }

    pub(crate) fn increments(&self) -> BTreeMap<String, Decimal> {
        self.increments
            .lock()
            .expect("the fake increment lock")
            .clone()
    }

    pub(crate) fn redis_value(&self) -> Option<ReseedCacheValue> {
        self.redis.lock().expect("the fake redis lock").clone()
    }
}

impl ReseedCache for FakeCache {
    type Error = FakeError;

    fn redis_configured(&self) -> bool {
        self.redis_configured
    }

    async fn redis_get(&self, _counter_key: &str) -> Result<Option<ReseedCacheValue>, FakeError> {
        self.redis_gets.fetch_add(1, Ordering::Relaxed);
        let scripted = self
            .redis_answers
            .lock()
            .expect("the fake answer lock")
            .pop_front();
        if let Some(answer) = scripted {
            return match answer {
                RedisAnswer::Hit(value) => Ok(Some(value)),
                RedisAnswer::Miss => Ok(None),
                RedisAnswer::Fail => Err(FakeError),
            };
        }
        Ok(self.redis_value())
    }

    async fn redis_set_nx(
        &self,
        counter_key: &str,
        value: Decimal,
    ) -> Result<Option<bool>, FakeError> {
        self.redis_sets
            .lock()
            .expect("the fake redis set lock")
            .push((counter_key.to_owned(), value));
        let answer = *self.set_answer.lock().expect("the fake set answer lock");
        match answer {
            SetAnswer::Fail => Err(FakeError),
            SetAnswer::Stored => {
                *self.redis.lock().expect("the fake redis lock") =
                    Some(ReseedCacheValue::Number(value));
                Ok(Some(true))
            }
            SetAnswer::Taken => Ok(None),
        }
    }

    async fn memory_get(&self, counter_key: &str) -> Result<Option<ReseedCacheValue>, FakeError> {
        self.memory_gets.fetch_add(1, Ordering::Relaxed);
        if self.memory_get_fails.load(Ordering::Relaxed) {
            return Err(FakeError);
        }
        let scripted = self
            .memory_answers
            .lock()
            .expect("the fake answer lock")
            .pop_front();
        match scripted {
            Some(answer) => Ok(answer),
            None => Ok(self.memory_text(counter_key)),
        }
    }

    async fn memory_set(&self, counter_key: &str, value: Decimal) -> Result<(), FakeError> {
        if self.memory_set_fails.load(Ordering::Relaxed) {
            return Err(FakeError);
        }
        self.memory_sets
            .lock()
            .expect("the fake memory set lock")
            .push((counter_key.to_owned(), value));
        self.memory
            .lock()
            .expect("the fake memory lock")
            .insert(counter_key.to_owned(), ReseedCacheValue::Number(value));
        Ok(())
    }

    async fn increment_local(
        &self,
        counter_key: &str,
        increment: Decimal,
    ) -> Result<Option<Decimal>, FakeError> {
        if self.increment_fails.load(Ordering::Relaxed) {
            return Err(FakeError);
        }
        let base = self
            .increments
            .lock()
            .expect("the fake increment lock")
            .get(counter_key)
            .copied()
            .unwrap_or(Decimal::ZERO);
        sleep(self.increment_delay).await;
        let next = base.checked_add(increment).unwrap_or(Decimal::MAX);
        self.increments
            .lock()
            .expect("the fake increment lock")
            .insert(counter_key.to_owned(), next);
        Ok(Some(next))
    }
}

pub(crate) struct FakeBatch {
    pub(crate) answer: Option<CounterValue>,
    pub(crate) fails: bool,
    pub(crate) recorded: Mutex<Vec<(String, Decimal)>>,
}

impl FakeBatch {
    pub(crate) fn new(answer: Option<CounterValue>) -> Self {
        Self {
            answer,
            fails: false,
            recorded: Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn recorded(&self) -> Vec<(String, Decimal)> {
        self.recorded.lock().expect("the fake batch lock").clone()
    }
}

impl ActiveSpendBatch for FakeBatch {
    async fn read(
        &self,
        _counter_key: &str,
    ) -> Result<Option<CounterValue>, SpendCounterBatchError> {
        if self.fails {
            return Err(SpendCounterBatchError::Cancelled);
        }
        Ok(self.answer)
    }

    fn record(&self, counter_key: &str, value: Decimal) {
        self.recorded
            .lock()
            .expect("the fake batch lock")
            .push((counter_key.to_owned(), value));
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

pub(crate) struct Services {
    pub(crate) tasks: TaskTracker,
    pub(crate) gate: Arc<LoopBoundSemaphore>,
    pub(crate) stall_tracker: Arc<DbLookupStallTracker>,
}

pub(crate) fn services(gate_slots: usize) -> Services {
    Services {
        tasks: TaskTracker::new(),
        gate: Arc::new(LoopBoundSemaphore {
            value: gate_slots,
            slot: Mutex::new(None),
        }),
        stall_tracker: Arc::new(DbLookupStallTracker {
            clock: SystemClock,
            last_hit: Mutex::new(None),
        }),
    }
}

pub(crate) fn reseed_with(
    rows: Option<Arc<FakeRows>>,
    services: Services,
    deadline_seconds: f64,
) -> SpendCounterReseed<FakeRows> {
    SpendCounterReseed {
        rows,
        locks: LockRegistry::from(10_000),
        clock: Box::new(FixedClock {
            now: Utc
                .with_ymd_and_hms(2026, 11, 18, 12, 0, 0)
                .single()
                .expect("a fixture instant"),
        }),
        durations: Box::new(FakeDurations),
        tasks: services.tasks,
        gate: services.gate,
        stall_tracker: services.stall_tracker,
        deadline_seconds,
    }
}

pub(crate) fn reseed(rows: FakeRows) -> SpendCounterReseed<FakeRows> {
    reseed_with(Some(Arc::new(rows)), services(25), 5.0)
}

#[derive(Default)]
struct LogFields(Option<String>);

impl tracing::field::Visit for LogFields {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.0 = Some(value.to_owned());
        }
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = Some(format!("{value:?}"));
        }
    }
}

#[derive(Clone, Default)]
struct MessageLog(Arc<Mutex<Vec<String>>>);

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for MessageLog {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
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
    _guard: tracing::subscriber::DefaultGuard,
}

impl Logs {
    pub(crate) fn capture() -> Self {
        let messages = MessageLog::default();
        let subscriber = tracing_subscriber::layer::SubscriberExt::with(
            tracing_subscriber::registry(),
            messages.clone(),
        );
        Self {
            messages,
            _guard: tracing::subscriber::set_default(subscriber),
        }
    }

    pub(crate) fn contains(&self, text: &str) -> bool {
        self.messages
            .0
            .lock()
            .is_ok_and(|messages| messages.iter().any(|message| message.contains(text)))
    }

    pub(crate) fn count(&self) -> usize {
        self.messages.0.lock().map_or(0, |messages| messages.len())
    }
}

struct SilentHooks;

impl ServiceHooks for SilentHooks {
    fn success(&self, _event: &ServiceEvent) {}

    fn failure(&self, _event: &ServiceEvent, _error: &(dyn Error + Send + Sync)) {}
}

// a gate that refuses every call, so the Redis tier fails before any socket is opened
struct RefusingGate;

impl BreakerGate for RefusingGate {
    fn enter(&self, name: &'static str) -> Result<BreakerAdmission, BreakerOpen> {
        Err(BreakerOpen { name })
    }

    fn exit(&self, _admission: BreakerAdmission) {}

    fn record_failure(&self, _error: &(dyn Error + Send + Sync)) {}

    fn swallowed(&self, _admission: &BreakerAdmission, _error: &(dyn Error + Send + Sync)) {}
}

pub(crate) async fn refusing_redis_cache() -> Arc<RedisCache> {
    let overrides: RedisOverrides = serde_json::from_str(
        r#"{"host":"127.0.0.1","port":1,"socket_timeout":1.0,"socket_connect_timeout":1.0}"#,
    )
    .expect("the Redis overrides parse");
    let options = RedisCacheOptions {
        overrides,
        family: RedisCacheFamily::Standalone,
        namespace: None,
        default_ttl: 60,
        redis_flush_size: None,
        default_major_version: 7,
        factory: Arc::new(LiveClientFactory::new(None)),
        store: Arc::new(RedisClientStore::new()),
        hooks: Arc::new(SilentHooks),
        gate: Arc::new(RefusingGate),
    };
    let env = BTreeMap::<String, String>::new();
    Arc::new(
        connect(options, &env)
            .await
            .expect("the dead Redis still builds a cache"),
    )
}
