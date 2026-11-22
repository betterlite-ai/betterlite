use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::future::{Future, ready};
use std::io;
use std::str::from_utf8;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use betterlite_db::budget_permission_rows::{JsonData, Timestamp};
use betterlite_db::database_failures::gate::{
    DbLookupStallTracker, LoopBoundSemaphore, SystemClock as DbSystemClock,
};
use betterlite_db::spend_rows::rows::{
    SpendCounterRows as DbSpendCounterRows, SpendRow as DbSpendRow, SpendRowLookup,
};
use chrono::{DateTime, FixedOffset, Utc};
use rust_decimal::Decimal;
use tokio_util::task::TaskTracker;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use crate::budget_counter::CounterEntityType;
use crate::budget_primitives::window_start::DurationParser;
use crate::model_budget_limiter::clock::SystemClock;
use crate::spend_counter_batch::batch::{CounterValue, SpendCounterBatchError};
use crate::spend_reads::SpendReads;
use crate::spend_reads::batch::{NoSpendBatch, SpendBatch};
use crate::spend_reads::cache::{SPEND_DB_FLOOR_MARKER_PREFIX, SpendReadCache};
use crate::spend_reads::request::{SpendReadRequest, SpendWindow};
use crate::spend_reseed::cache::{ActiveSpendBatch, ReseedCache};
use crate::spend_reseed::locks::LockRegistry;
use crate::spend_reseed::reseed::SpendCounterReseed;
use crate::spend_reseed::rows::{
    SpendAggregateRow, SpendCounterRows, WindowEntity, WindowSpendRow,
};
use crate::spend_reseed::values::ReseedCacheValue;

pub(crate) const KEY: &str = "spend:key:token";

#[derive(Debug, thiserror::Error)]
#[error("the fake store failed")]
pub(crate) struct FakeError;

type TierAnswer = Result<Option<ReseedCacheValue>, FakeError>;

pub(crate) fn decimal(text: &str) -> Decimal {
    text.parse().expect("a fixture decimal parses")
}

pub(crate) fn marker_key(counter_key: &str) -> String {
    format!("{SPEND_DB_FLOOR_MARKER_PREFIX}{counter_key}")
}

pub(crate) fn aware(text: &str) -> Timestamp {
    Timestamp::Aware(
        DateTime::<FixedOffset>::parse_from_rfc3339(text).expect("a fixture timestamp parses"),
    )
}

pub(crate) fn json(text: &str) -> JsonData {
    serde_json::from_str(text).expect("a fixture JSON value parses")
}

pub(crate) fn aggregate(spend: &str) -> SpendAggregateRow {
    SpendAggregateRow::Attribute {
        sum: Some(JsonData::Object(BTreeMap::from([(
            "spend".to_owned(),
            json(spend),
        )]))),
    }
}

pub(crate) fn request(
    counter_key: &str,
    fallback_spend: Decimal,
    max_budget: Option<Decimal>,
) -> SpendReadRequest<'_> {
    SpendReadRequest {
        counter_key,
        fallback_spend,
        max_budget,
        window: None,
        fallback_authoritative: false,
    }
}

pub(crate) fn key_window(entity: &CounterEntityType, start: Timestamp) -> SpendWindow<'_> {
    SpendWindow {
        entity_type: entity,
        entity_id: "token",
        window_duration: Some("1h"),
        window_start: start,
    }
}

pub(crate) fn window_row(start: &str, spend: &str) -> WindowSpendRow {
    WindowSpendRow {
        window_start: aware(start),
        spend: Some(json(spend)),
    }
}

pub(crate) fn written(
    writes: &[(String, Decimal, Option<f64>)],
    key: &str,
) -> Option<(Decimal, Option<f64>)> {
    writes
        .iter()
        .find(|(written_key, _value, _ttl)| written_key.as_str() == key)
        .map(|(_key, value, ttl)| (*value, *ttl))
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) enum RowAnswer {
    #[default]
    Missing,
    Null,
    Spent(Decimal),
}

pub(crate) struct RaceWrite {
    cache: Arc<FakeCache>,
    marker_key: String,
    floor: ReseedCacheValue,
}

#[derive(Default)]
pub(crate) struct FakeRows {
    entity: Mutex<RowAnswer>,
    window: Mutex<Option<WindowSpendRow>>,
    logs: Mutex<Vec<SpendAggregateRow>>,
    entity_fails: AtomicBool,
    window_fails: AtomicBool,
    log_fails: AtomicBool,
    entity_reads: Mutex<Vec<String>>,
    window_reads: Mutex<Vec<String>>,
    log_reads: AtomicUsize,
    race: Mutex<Option<RaceWrite>>,
}

impl FakeRows {
    pub(crate) fn set_entity(&self, answer: RowAnswer) {
        *self.entity.lock().unwrap_or_else(PoisonError::into_inner) = answer;
    }

    pub(crate) fn set_window(&self, row: WindowSpendRow) {
        *self.window.lock().unwrap_or_else(PoisonError::into_inner) = Some(row);
    }

    pub(crate) fn set_logs(&self, rows: Vec<SpendAggregateRow>) {
        *self.logs.lock().unwrap_or_else(PoisonError::into_inner) = rows;
    }

    pub(crate) fn fail_entity(&self) {
        self.entity_fails.store(true, Ordering::Relaxed);
    }

    pub(crate) fn fail_window(&self) {
        self.window_fails.store(true, Ordering::Relaxed);
    }

    pub(crate) fn fail_logs(&self) {
        self.log_fails.store(true, Ordering::Relaxed);
    }

    pub(crate) fn entity_reads(&self) -> Vec<String> {
        self.entity_reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn window_reads(&self) -> Vec<String> {
        self.window_reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn log_reads(&self) -> usize {
        self.log_reads.load(Ordering::Relaxed)
    }

    fn row_answer(&self, lookup: SpendRowLookup<'_>) -> Result<Option<DbSpendRow>, FakeError> {
        self.entity_reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(lookup_label(lookup));
        if self.entity_fails.load(Ordering::Relaxed) {
            return Err(FakeError);
        }
        let race = self
            .race
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(write) = race {
            write.cache.seed_memory(&write.marker_key, write.floor);
        }
        let answer = *self.entity.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(match answer {
            RowAnswer::Missing => None,
            RowAnswer::Null => Some(DbSpendRow { spend: None }),
            RowAnswer::Spent(amount) => Some(DbSpendRow {
                spend: Some(amount),
            }),
        })
    }

    fn window_answer(
        &self,
        window_entity: WindowEntity,
        window_id: &str,
        duration: &str,
    ) -> Result<Option<WindowSpendRow>, FakeError> {
        self.window_reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!("{window_entity:?}:{window_id}:{duration}"));
        if self.window_fails.load(Ordering::Relaxed) {
            return Err(FakeError);
        }
        Ok(self
            .window
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone())
    }

    fn log_answer(&self) -> Result<Vec<SpendAggregateRow>, FakeError> {
        self.log_reads.fetch_add(1, Ordering::Relaxed);
        if self.log_fails.load(Ordering::Relaxed) {
            return Err(FakeError);
        }
        Ok(self
            .logs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone())
    }
}

fn lookup_label(lookup: SpendRowLookup<'_>) -> String {
    match lookup {
        SpendRowLookup::Key(id) => format!("key:{id}"),
        SpendRowLookup::TeamMember { user_id, team_id } => format!("member:{user_id}:{team_id}"),
        SpendRowLookup::Team(id) => format!("team:{id}"),
        SpendRowLookup::User(id) => format!("user:{id}"),
        SpendRowLookup::Organization(id) => format!("org:{id}"),
        SpendRowLookup::Project(id) => format!("project:{id}"),
        SpendRowLookup::EndUser(id) => format!("end_user:{id}"),
    }
}

impl DbSpendCounterRows for FakeRows {
    type Error = FakeError;

    fn spend_row(
        &self,
        lookup: SpendRowLookup<'_>,
    ) -> impl Future<Output = Result<Option<DbSpendRow>, FakeError>> + Send {
        ready(self.row_answer(lookup))
    }
}

impl SpendCounterRows for FakeRows {
    type Error = FakeError;

    fn window_row(
        &self,
        entity: WindowEntity,
        entity_id: &str,
        window_duration: &str,
    ) -> impl Future<Output = Result<Option<WindowSpendRow>, FakeError>> + Send {
        ready(self.window_answer(entity, entity_id, window_duration))
    }

    fn window_log_rows(
        &self,
        _entity: WindowEntity,
        _entity_id: &str,
        _window_start: DateTime<Utc>,
    ) -> impl Future<Output = Result<Vec<SpendAggregateRow>, FakeError>> + Send {
        ready(self.log_answer())
    }
}

pub(crate) struct FakeCache {
    redis_configured: bool,
    redis: Mutex<BTreeMap<String, ReseedCacheValue>>,
    memory: Mutex<BTreeMap<String, ReseedCacheValue>>,
    redis_fails: AtomicBool,
    memory_read_fails: AtomicBool,
    memory_write_fails: AtomicBool,
    set_max_fails: AtomicBool,
    set_max_none: AtomicBool,
    memory_writes: Mutex<Vec<(String, Decimal, Option<f64>)>>,
    set_max_calls: Mutex<Vec<(String, Decimal)>>,
    nx_writes: Mutex<Vec<(String, Decimal)>>,
    events: Arc<Mutex<Vec<String>>>,
}

impl FakeCache {
    pub(crate) fn new(redis_configured: bool, events: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            redis_configured,
            redis: Mutex::new(BTreeMap::new()),
            memory: Mutex::new(BTreeMap::new()),
            redis_fails: AtomicBool::new(false),
            memory_read_fails: AtomicBool::new(false),
            memory_write_fails: AtomicBool::new(false),
            set_max_fails: AtomicBool::new(false),
            set_max_none: AtomicBool::new(false),
            memory_writes: Mutex::new(Vec::new()),
            set_max_calls: Mutex::new(Vec::new()),
            nx_writes: Mutex::new(Vec::new()),
            events,
        }
    }

    pub(crate) fn seed_redis(&self, key: &str, value: ReseedCacheValue) {
        self.redis
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value);
    }

    pub(crate) fn seed_memory(&self, key: &str, value: ReseedCacheValue) {
        self.memory
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value);
    }

    pub(crate) fn fail_redis(&self) {
        self.redis_fails.store(true, Ordering::Relaxed);
    }

    pub(crate) fn fail_memory_reads(&self) {
        self.memory_read_fails.store(true, Ordering::Relaxed);
    }

    pub(crate) fn fail_memory_writes(&self) {
        self.memory_write_fails.store(true, Ordering::Relaxed);
    }

    pub(crate) fn fail_set_max(&self) {
        self.set_max_fails.store(true, Ordering::Relaxed);
    }

    pub(crate) fn set_max_answers_nothing(&self) {
        self.set_max_none.store(true, Ordering::Relaxed);
    }

    pub(crate) fn stored_memory(&self, key: &str) -> Option<ReseedCacheValue> {
        self.memory
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned()
    }

    pub(crate) fn stored_redis(&self, key: &str) -> Option<ReseedCacheValue> {
        self.redis
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned()
    }

    pub(crate) fn memory_writes(&self) -> Vec<(String, Decimal, Option<f64>)> {
        self.memory_writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn set_max_calls(&self) -> Vec<(String, Decimal)> {
        self.set_max_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn nx_writes(&self) -> Vec<(String, Decimal)> {
        self.nx_writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record(&self, event: String) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }

    fn tier_answer(
        &self,
        event: String,
        fails: &AtomicBool,
        stored: Option<ReseedCacheValue>,
    ) -> TierAnswer {
        self.record(event);
        if fails.load(Ordering::Relaxed) {
            return Err(FakeError);
        }
        Ok(stored)
    }

    fn store_memory(
        &self,
        key: &str,
        value: Decimal,
        ttl_seconds: Option<f64>,
    ) -> Result<(), FakeError> {
        self.record(format!("memory_set:{key}"));
        if self.memory_write_fails.load(Ordering::Relaxed) {
            return Err(FakeError);
        }
        self.memory_writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((key.to_owned(), value, ttl_seconds));
        self.seed_memory(key, ReseedCacheValue::Number(value));
        Ok(())
    }
}

impl ReseedCache for FakeCache {
    type Error = FakeError;

    fn redis_configured(&self) -> bool {
        self.redis_configured
    }

    fn redis_get(&self, counter_key: &str) -> impl Future<Output = TierAnswer> + Send {
        ready(self.tier_answer(
            format!("redis_get:{counter_key}"),
            &self.redis_fails,
            self.stored_redis(counter_key),
        ))
    }

    fn redis_set_nx(
        &self,
        counter_key: &str,
        value: Decimal,
    ) -> impl Future<Output = Result<Option<bool>, FakeError>> + Send {
        self.record(format!("set_nx:{counter_key}"));
        self.nx_writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((counter_key.to_owned(), value));
        let seeded = {
            let mut redis = self.redis.lock().unwrap_or_else(PoisonError::into_inner);
            match redis.entry(counter_key.to_owned()) {
                Entry::Occupied(_) => false,
                Entry::Vacant(slot) => {
                    slot.insert(ReseedCacheValue::Number(value));
                    true
                }
            }
        };
        ready(Ok(Some(seeded)))
    }

    fn memory_get(&self, counter_key: &str) -> impl Future<Output = TierAnswer> + Send {
        ready(self.tier_answer(
            format!("memory_get:{counter_key}"),
            &self.memory_read_fails,
            self.stored_memory(counter_key),
        ))
    }

    fn memory_set(
        &self,
        counter_key: &str,
        value: Decimal,
    ) -> impl Future<Output = Result<(), FakeError>> + Send {
        ready(self.store_memory(counter_key, value, None))
    }

    // the read path never increments; B1736's traffic does
    fn increment_local(
        &self,
        _counter_key: &str,
        _increment: Decimal,
    ) -> impl Future<Output = Result<Option<Decimal>, FakeError>> + Send {
        ready(Ok(None))
    }
}

impl SpendReadCache for FakeCache {
    fn redis_set_max(
        &self,
        counter_key: &str,
        value: Decimal,
    ) -> impl Future<Output = Result<Option<Decimal>, FakeError>> + Send {
        self.record(format!("set_max:{counter_key}:{value}"));
        self.set_max_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((counter_key.to_owned(), value));
        if self.set_max_fails.load(Ordering::Relaxed) {
            return ready(Err(FakeError));
        }
        if self.set_max_none.load(Ordering::Relaxed) {
            return ready(Ok(None));
        }
        let current = self.stored_redis(counter_key);
        let result = match current {
            Some(ReseedCacheValue::Number(amount)) if amount >= value => amount,
            Some(
                ReseedCacheValue::Number(_)
                | ReseedCacheValue::Text(_)
                | ReseedCacheValue::Boolean(_)
                | ReseedCacheValue::Unreadable,
            )
            | None => {
                self.seed_redis(counter_key, ReseedCacheValue::Number(value));
                value
            }
        };
        ready(Ok(Some(result)))
    }

    fn memory_set_ttl(
        &self,
        counter_key: &str,
        value: Decimal,
        ttl_seconds: Option<f64>,
    ) -> Result<(), FakeError> {
        self.store_memory(counter_key, value, ttl_seconds)
    }
}

pub(crate) struct FakeBatch {
    answer: Mutex<Option<CounterValue>>,
    fails: AtomicBool,
    recorded: Mutex<Vec<(String, Decimal)>>,
    forgotten: Mutex<Vec<String>>,
    events: Arc<Mutex<Vec<String>>>,
}

impl FakeBatch {
    pub(crate) fn new(events: Arc<Mutex<Vec<String>>>) -> Self {
        let answer = Mutex::new(None);
        let fails = AtomicBool::new(false);
        let recorded = Mutex::new(Vec::new());
        let forgotten = Mutex::new(Vec::new());
        Self {
            answer,
            fails,
            recorded,
            forgotten,
            events,
        }
    }

    pub(crate) fn set_answer(&self, answer: Option<CounterValue>) {
        *self.answer.lock().unwrap_or_else(PoisonError::into_inner) = answer;
    }

    pub(crate) fn fail(&self) {
        self.fails.store(true, Ordering::Relaxed);
    }

    pub(crate) fn recorded(&self) -> Vec<(String, Decimal)> {
        self.recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn forgotten(&self) -> Vec<String> {
        self.forgotten
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn record_event(&self, event: String) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }
}

impl ActiveSpendBatch for FakeBatch {
    fn read(
        &self,
        counter_key: &str,
    ) -> impl Future<Output = Result<Option<CounterValue>, SpendCounterBatchError>> + Send {
        self.record_event(format!("batch_read:{counter_key}"));
        if self.fails.load(Ordering::Relaxed) {
            return ready(Err(SpendCounterBatchError::Cancelled));
        }
        let answer = *self.answer.lock().unwrap_or_else(PoisonError::into_inner);
        ready(Ok(answer))
    }

    fn record(&self, counter_key: &str, value: Decimal) {
        self.record_event(format!("batch_record:{counter_key}:{value}"));
        self.recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((counter_key.to_owned(), value));
    }
}

impl SpendBatch for FakeBatch {
    fn forget(&self, counter_key: &str) {
        self.record_event(format!("batch_forget:{counter_key}"));
        self.forgotten
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(counter_key.to_owned());
    }
}

pub(crate) struct FakeDurations;

impl DurationParser for FakeDurations {
    fn seconds(&self, spec: &str, _now: DateTime<Utc>) -> Option<i64> {
        let hours = spec.strip_suffix('h')?;
        hours.parse::<i64>().ok()?.checked_mul(3_600)
    }
}

pub(crate) struct Fixture {
    pub(crate) rows: Arc<FakeRows>,
    pub(crate) cache: Arc<FakeCache>,
    pub(crate) batch: Arc<FakeBatch>,
    pub(crate) events: Arc<Mutex<Vec<String>>>,
    pub(crate) reseed: SpendCounterReseed<FakeRows>,
}

impl Fixture {
    pub(crate) fn new(redis_configured: bool) -> Self {
        let events = Arc::new(Mutex::new(Vec::new()));
        let rows = Arc::new(FakeRows::default());
        let cache = Arc::new(FakeCache::new(redis_configured, Arc::clone(&events)));
        let batch = Arc::new(FakeBatch::new(Arc::clone(&events)));
        let reseed = SpendCounterReseed {
            rows: Some(Arc::clone(&rows)),
            locks: LockRegistry::from(1_000),
            clock: Box::new(SystemClock {
                started: Instant::now(),
            }),
            durations: Arc::new(FakeDurations),
            tasks: TaskTracker::new(),
            gate: Arc::new(LoopBoundSemaphore {
                value: 4,
                slot: Mutex::new(None),
            }),
            stall_tracker: Arc::new(DbLookupStallTracker {
                clock: DbSystemClock,
                last_hit: Mutex::new(None),
            }),
            deadline_seconds: 5.0,
        };
        Self {
            rows,
            cache,
            batch,
            events,
            reseed,
        }
    }

    pub(crate) fn reads(
        &self,
        fail_closed: bool,
    ) -> SpendReads<'_, FakeRows, FakeCache, FakeBatch> {
        self.reads_with(Some(self.batch.as_ref()), fail_closed)
    }

    pub(crate) fn reads_without_batch(
        &self,
        fail_closed: bool,
    ) -> SpendReads<'_, FakeRows, FakeCache, NoSpendBatch> {
        self.reads_with(None, fail_closed)
    }

    fn reads_with<'reads, Batch>(
        &'reads self,
        batch: Option<&'reads Batch>,
        fail_closed: bool,
    ) -> SpendReads<'reads, FakeRows, FakeCache, Batch>
    where
        Batch: SpendBatch,
    {
        SpendReads {
            reseed: &self.reseed,
            cache: self.cache.as_ref(),
            batch,
            fail_closed_budget_enforcement: fail_closed,
        }
    }

    pub(crate) fn events(&self) -> Vec<String> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn race_marker(&self, counter_key: &str, floor: ReseedCacheValue) {
        *self
            .rows
            .race
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(RaceWrite {
            cache: Arc::clone(&self.cache),
            marker_key: marker_key(counter_key),
            floor,
        });
    }
}

#[derive(Clone, Default)]
pub(crate) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Captured {
    pub(crate) fn install(&self) -> DefaultGuard {
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .without_time()
            .with_target(false)
            .with_max_level(tracing::Level::TRACE)
            .with_writer(self.clone())
            .finish();
        set_default(subscriber)
    }

    pub(crate) fn has(&self, needle: &str) -> bool {
        let log = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        from_utf8(&log).is_ok_and(|text| text.contains(needle))
    }
}

impl io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut log = self
            .0
            .lock()
            .map_err(|_error| io::Error::other("the captured log lock"))?;
        log.extend_from_slice(buf);
        drop(log);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for Captured {
    type Writer = Self;

    fn make_writer(&self) -> Self::Writer {
        self.clone()
    }
}
