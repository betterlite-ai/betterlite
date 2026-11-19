#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::future::pending;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_cache::backend::BackendError;
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt::MakeWriter;

use crate::auth_cache::keys::{Partition, partition};
use crate::auth_prefetch::db::{PrefetchDatabase, PrefetchDatabaseError};
use crate::auth_prefetch::entries::AuthObjectRefs;
use crate::auth_prefetch::fill::PrefetchClock;
use crate::auth_prefetch::memory::{MemoryWriteError, PrefetchCache, PrefetchMemory};
use crate::auth_prefetch::orchestrator::AdmissionCounterBinding;
use crate::auth_prefetch::redis_rows::{PrefetchRedis, RedisRowsError};
use crate::auth_prefetch::rows::{RowKind, RowValues};
use crate::resolved_key::auth::{AuthInput, UserAPIKeyAuth};

pub(super) type Events = Arc<Mutex<Vec<String>>>;

pub(super) fn events() -> Events {
    Arc::new(Mutex::new(Vec::new()))
}

pub(super) fn record(events: &Events, entry: &str) {
    events
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(entry.to_owned());
}

pub(super) fn recorded(events: &Events) -> Vec<String> {
    events
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

pub(super) fn json(text: &str) -> JsonData {
    serde_json::from_str(text).expect("the fixture is valid json")
}

pub(super) fn auth(entries: &[(&str, JsonData)]) -> UserAPIKeyAuth {
    let fields: BTreeMap<String, JsonData> = entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect();
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).expect("the auth fixture builds")
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct MemoryWrite {
    pub(super) key: String,
    pub(super) value: CacheValue,
    pub(super) ttl: Option<f64>,
}

pub(super) struct WriteLog<T> {
    entries: Mutex<Vec<T>>,
}

impl<T> Default for WriteLog<T> {
    fn default() -> Self {
        Self {
            entries: Mutex::default(),
        }
    }
}

impl<T: Clone> WriteLog<T> {
    pub(super) fn push(&self, entry: T) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entry);
    }

    pub(super) fn all(&self) -> Vec<T> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Default)]
pub(super) struct FakeMemory {
    values: Mutex<BTreeMap<String, CacheValue>>,
    writes: WriteLog<MemoryWrite>,
    reads: Mutex<Vec<String>>,
    failing: Mutex<BTreeSet<String>>,
}

impl FakeMemory {
    pub(super) fn seed(&self, key: &str, value: CacheValue) {
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value);
    }

    pub(super) fn fail_writes_for(&self, key: &str) {
        self.failing
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned());
    }

    pub(super) fn writes(&self) -> Vec<MemoryWrite> {
        self.writes.all()
    }

    pub(super) fn write_keys(&self) -> Vec<String> {
        self.writes().into_iter().map(|write| write.key).collect()
    }

    pub(super) fn reads(&self) -> Vec<String> {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn get_value(&self, key: &str) -> Option<CacheValue> {
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned()
    }
}

impl PrefetchMemory for FakeMemory {
    fn get(&self, key: &str) -> Option<CacheValue> {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(key.to_owned());
        self.get_value(key)
    }

    fn set(&self, key: &str, value: CacheValue, ttl: Option<f64>) -> Result<(), MemoryWriteError> {
        if self
            .failing
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(key)
        {
            return Err(MemoryWriteError(BackendError::Unsupported(
                "the test memory refuses this key",
            )));
        }
        self.writes.push(MemoryWrite {
            key: key.to_owned(),
            value: value.clone(),
            ttl,
        });
        self.seed(key, value);
        Ok(())
    }
}

// the two partitions the cache keys select between, with one optional request pipeline
pub(super) struct FakeCache {
    pub(super) ordinary: FakeMemory,
    pub(super) key_object: FakeMemory,
    pub(super) management_ttl: f64,
    pub(super) default_memory_ttl: Option<f64>,
    pub(super) redis: Option<Arc<FakeRedis>>,
}

impl FakeCache {
    pub(super) fn new(management_ttl: f64) -> Self {
        Self {
            ordinary: FakeMemory::default(),
            key_object: FakeMemory::default(),
            management_ttl,
            default_memory_ttl: None,
            redis: None,
        }
    }

    pub(super) fn with_redis(management_ttl: f64, redis: Arc<FakeRedis>) -> Self {
        Self {
            redis: Some(redis),
            ..Self::new(management_ttl)
        }
    }
}

impl PrefetchCache for FakeCache {
    fn ordinary_memory(&self) -> &dyn PrefetchMemory {
        &self.ordinary
    }

    fn memory_for(&self, key: &str) -> &dyn PrefetchMemory {
        match partition(key) {
            Partition::Ordinary => &self.ordinary,
            Partition::KeyObject => &self.key_object,
        }
    }

    fn management_ttl(&self) -> f64 {
        self.management_ttl
    }

    fn default_memory_ttl(&self) -> Option<f64> {
        self.default_memory_ttl
    }

    fn redis(&self) -> Option<Arc<dyn PrefetchRedis>> {
        self.redis
            .clone()
            .map(|redis| -> Arc<dyn PrefetchRedis> { redis })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ReadFailure {
    Batch(&'static str),
    Direct(&'static str),
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct QueueWrite {
    pub(super) key: String,
    pub(super) value: Json,
    pub(super) ttl: Option<f64>,
}

impl QueueWrite {
    pub(super) fn new(key: &str, value: &Json, ttl: Option<f64>) -> Self {
        Self {
            key: key.to_owned(),
            value: value.clone(),
            ttl,
        }
    }
}

#[derive(Default)]
pub(super) struct FakeRedis {
    values: Mutex<BTreeMap<String, Json>>,
    reads: Mutex<Vec<Vec<String>>>,
    read_failure: Mutex<Option<ReadFailure>>,
    non_object: AtomicBool,
    batch: AtomicBool,
    fail_pipeline: AtomicBool,
    queue_failure: Mutex<Option<String>>,
    queued: WriteLog<QueueWrite>,
    pipelines: Mutex<Vec<Vec<QueueWrite>>>,
}

impl FakeRedis {
    pub(super) fn batch() -> Self {
        let redis = Self::default();
        redis.batch.store(true, Ordering::SeqCst);
        redis
    }

    pub(super) fn seed(&self, key: &str, value: Json) {
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value);
    }

    pub(super) fn fail_read(&self, failure: ReadFailure) {
        *self
            .read_failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(failure);
    }

    pub(super) fn fail_writes(&self) {
        self.fail_pipeline.store(true, Ordering::SeqCst);
    }

    pub(super) fn return_non_object(&self) {
        self.non_object.store(true, Ordering::SeqCst);
    }

    pub(super) fn fail_queue_for(&self, key: &str) {
        *self
            .queue_failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(key.to_owned());
    }

    pub(super) fn reads(&self) -> Vec<Vec<String>> {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn queued(&self) -> Vec<QueueWrite> {
        self.queued.all()
    }

    pub(super) fn pipelines(&self) -> Vec<Vec<QueueWrite>> {
        self.pipelines
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl PrefetchRedis for FakeRedis {
    async fn read(&self, keys: &[String]) -> Result<Json, RedisRowsError> {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(keys.to_vec());
        let failure = *self
            .read_failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(refusal) = failure {
            return Err(match refusal {
                ReadFailure::Batch(message) => RedisRowsError::Batch {
                    message: message.to_owned(),
                },
                ReadFailure::Direct(message) => RedisRowsError::Direct {
                    message: message.to_owned(),
                },
            });
        }
        if self.non_object.load(Ordering::SeqCst) {
            return Ok(Json::List(Vec::new()));
        }
        let values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        let mut found = IndexMap::new();
        for key in keys {
            if let Some(value) = values.get(key) {
                found.insert(key.clone(), value.clone());
            }
        }
        Ok(Json::Object(found))
    }

    fn has_batch(&self) -> bool {
        self.batch.load(Ordering::SeqCst)
    }

    async fn set_pipeline(
        &self,
        entries: &[(String, Json, Option<f64>)],
    ) -> Result<(), RedisRowsError> {
        self.pipelines
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(entries.iter().map(queue_write).collect());
        if self.fail_pipeline.load(Ordering::SeqCst) {
            return Err(RedisRowsError::Write {
                message: "the test pipeline failed".to_owned(),
            });
        }
        Ok(())
    }

    fn queue_set(&self, key: &str, value: &Json, ttl: Option<f64>) -> Result<(), RedisRowsError> {
        let failure = self
            .queue_failure
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .filter(|failure| failure == key);
        if let Some(refusal) = failure {
            return Err(RedisRowsError::Write {
                message: format!("the test queue refused {refusal}"),
            });
        }
        self.queued.push(QueueWrite::new(key, value, ttl));
        Ok(())
    }
}

fn queue_write(entry: &(String, Json, Option<f64>)) -> QueueWrite {
    QueueWrite::new(&entry.0, &entry.1, entry.2)
}

#[derive(Default)]
pub(super) struct FakeDatabase {
    pub(super) rows: RowValues,
    calls: Mutex<Vec<Vec<RowKind>>>,
    pub(super) events: Option<Events>,
    dropped: Option<Arc<AtomicBool>>,
}

impl FakeDatabase {
    pub(super) fn with(rows: &[(RowKind, JsonData)]) -> Self {
        let mut values = RowValues::default();
        for (kind, value) in rows {
            values.insert(*kind, value.clone());
        }
        Self::assemble(values, None)
    }

    // a lookup that never answers, holding a flag that proves the future was dropped
    pub(super) fn hanging(dropped: Arc<AtomicBool>) -> Self {
        Self::assemble(RowValues::default(), Some(dropped))
    }

    fn assemble(rows: RowValues, dropped: Option<Arc<AtomicBool>>) -> Self {
        Self {
            rows,
            calls: Mutex::default(),
            events: None,
            dropped,
        }
    }

    pub(super) fn record_events(&mut self, events: Events) {
        self.events = Some(events);
    }

    pub(super) fn calls(&self) -> Vec<Vec<RowKind>> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl PrefetchDatabase for FakeDatabase {
    async fn fetch_rows(
        &self,
        _refs: &AuthObjectRefs,
        kinds: &[RowKind],
    ) -> Result<RowValues, PrefetchDatabaseError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(kinds.to_vec());
        if let Some(events) = &self.events {
            record(events, "db");
        }
        if let Some(dropped) = &self.dropped {
            let _held = DropFlag(Arc::clone(dropped));
            return pending().await;
        }
        Ok(self.rows.clone())
    }
}

pub(super) struct DropFlag(pub(super) Arc<AtomicBool>);

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct FixedClock {
    pub(super) now: f64,
}

impl PrefetchClock for FixedClock {
    fn now(&self) -> f64 {
        self.now
    }
}

#[derive(Clone, Default)]
pub(super) struct Logs(Arc<Mutex<Vec<u8>>>);

impl Logs {
    pub(super) fn capture() -> (Self, DefaultGuard) {
        let logs = Self::default();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .without_time()
            .with_target(false)
            .with_max_level(tracing::Level::TRACE)
            .with_writer(logs.clone())
            .finish();
        (logs, set_default(subscriber))
    }

    pub(super) fn text(&self) -> String {
        String::from_utf8(
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
        .expect("the captured logs are UTF-8")
    }
}

impl io::Write for Logs {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl MakeWriter<'_> for Logs {
    type Writer = Self;

    fn make_writer(&self) -> Self {
        self.clone()
    }
}

#[derive(Default)]
pub(super) struct RecordingBinder {
    pub(super) bindings: Mutex<Vec<Option<String>>>,
    pub(super) events: Option<Events>,
}

impl RecordingBinder {
    pub(super) fn new(events: &Events) -> Self {
        Self {
            bindings: Mutex::new(Vec::new()),
            events: Some(Arc::clone(events)),
        }
    }

    pub(super) fn bindings(&self) -> Vec<Option<String>> {
        self.bindings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl AdmissionCounterBinding for RecordingBinder {
    fn bind_admission_counters(&self, _auth: &UserAPIKeyAuth, end_user_id: Option<&str>) {
        self.bindings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(end_user_id.map(str::to_owned));
        if let Some(events) = &self.events {
            record(events, "bind");
        }
    }
}
