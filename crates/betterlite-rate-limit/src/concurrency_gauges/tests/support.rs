use std::collections::BTreeMap;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_redis::batch::error::BatchError;
use betterlite_redis::cache_primitives::protocol::RedisScalar;
use betterlite_redis::redis_counters::RedisCountersError;
use tokio::sync::Mutex as AsyncMutex;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt;

use crate::concurrency_gauges::ConcurrencyGauges;
use crate::concurrency_gauges::batch::{ReleaseBatch, ReleaseBatchSource, ReleaseSettledHook};
use crate::concurrency_gauges::cache::{GaugeCache, GaugeCacheValue};
use crate::fail_closed::FailClosedPolicy;
use crate::limiter_records::{
    CacheCounterValue, LimiterClock, ParallelRequestGauge, ParallelSlotAcquisition, ScriptReply,
};
use crate::rate_limit_window_store::{CounterScript, ScriptCall};
use crate::request_stash::{RequestStash, StashState};

pub(crate) fn key_list(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

pub(crate) fn gauge(counter_key: &str, limit: i64) -> ParallelRequestGauge {
    ParallelRequestGauge {
        counter_key: counter_key.to_owned(),
        limit,
        descriptor_key: "api-key".to_owned(),
    }
}

pub(crate) fn acquisition(slot_id: &str, counter_keys: &[&str]) -> ParallelSlotAcquisition {
    ParallelSlotAcquisition {
        slot_id: slot_id.to_owned(),
        counter_keys: key_list(counter_keys),
    }
}

pub(crate) fn registry(slots: &[(&str, f64)]) -> GaugeCacheValue {
    let mut entries = BTreeMap::new();
    for (slot, timestamp) in slots {
        entries.insert((*slot).to_owned(), *timestamp);
    }
    GaugeCacheValue::Registry(entries)
}

pub(crate) fn int_counter(value: i64) -> GaugeCacheValue {
    GaugeCacheValue::Counter(CacheCounterValue::Int(value))
}

pub(crate) fn stash_with(acquisition_value: ParallelSlotAcquisition) -> RequestStash {
    let stash = empty_stash();
    stash
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .parallel_slot = Some(acquisition_value);
    stash
}

pub(crate) fn empty_stash() -> RequestStash {
    RequestStash {
        state: Mutex::new(StashState::default()),
        parallel_slot_release_lock: AsyncMutex::new(()),
    }
}

pub(crate) fn gauges_with(cache: Arc<RecordingCache>, clock: Arc<FixedClock>) -> ConcurrencyGauges {
    ConcurrencyGauges {
        cache,
        clock,
        acquire_script: None,
        release_script: None,
        count_script: None,
        release_batch: None,
        fail_closed: FailClosedPolicy { enabled: false },
        check_and_increment_lock: AsyncMutex::new(()),
    }
}

pub(crate) struct FixedClock {
    at: SystemTime,
}

impl FixedClock {
    pub(crate) fn at(seconds: f64) -> Arc<Self> {
        let elapsed = Duration::try_from_secs_f64(seconds).expect("a valid clock time");
        let at = UNIX_EPOCH
            .checked_add(elapsed)
            .expect("a representable time");
        Arc::new(Self { at })
    }
}

impl LimiterClock for FixedClock {
    fn current_time(&self) -> SystemTime {
        self.at
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CacheWrite {
    pub(crate) key: String,
    pub(crate) value: GaugeCacheValue,
    pub(crate) ttl: Duration,
    pub(crate) local_only: bool,
}

pub(crate) struct RecordingCache {
    values: Mutex<BTreeMap<String, GaugeCacheValue>>,
    reads: Mutex<Vec<(String, bool)>>,
    batch_reads: Mutex<Vec<(Vec<String>, bool)>>,
    writes: Mutex<Vec<CacheWrite>>,
    batch_none: AtomicBool,
}

impl RecordingCache {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            values: Mutex::new(BTreeMap::new()),
            reads: Mutex::new(Vec::new()),
            batch_reads: Mutex::new(Vec::new()),
            writes: Mutex::new(Vec::new()),
            batch_none: AtomicBool::new(false),
        })
    }

    pub(crate) fn seed(&self, key: &str, value: GaugeCacheValue) {
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value);
    }

    pub(crate) fn fail_batch_reads(&self) {
        self.batch_none.store(true, Ordering::SeqCst);
    }

    pub(crate) fn reads(&self) -> Vec<(String, bool)> {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn batch_reads(&self) -> Vec<(Vec<String>, bool)> {
        self.batch_reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn writes(&self) -> Vec<CacheWrite> {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait::async_trait]
impl GaugeCache for RecordingCache {
    async fn get(&self, key: &str, local_only: bool) -> Option<GaugeCacheValue> {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((key.to_owned(), local_only));
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned()
    }

    async fn set(&self, key: &str, value: GaugeCacheValue, ttl: Duration, local_only: bool) {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(CacheWrite {
                key: key.to_owned(),
                value: value.clone(),
                ttl,
                local_only,
            });
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value);
    }

    async fn batch_get(
        &self,
        keys: &[String],
        local_only: bool,
    ) -> Option<Vec<Option<GaugeCacheValue>>> {
        self.batch_reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((keys.to_vec(), local_only));
        if self.batch_none.load(Ordering::SeqCst) {
            return None;
        }
        let values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        Some(keys.iter().map(|key| values.get(key).cloned()).collect())
    }
}

pub(crate) enum ReplyPlan {
    Values(Vec<CacheCounterValue>),
    Element(CacheCounterValue),
    Failure,
}

pub(crate) fn int_values(values: &[i64]) -> ReplyPlan {
    ReplyPlan::Values(values.iter().copied().map(CacheCounterValue::Int).collect())
}

pub(crate) struct PlannedScript {
    source: &'static str,
    plans: Vec<ReplyPlan>,
    calls: Mutex<Vec<ScriptCall>>,
}

impl PlannedScript {
    pub(crate) fn new(source: &'static str, plans: Vec<ReplyPlan>) -> Arc<Self> {
        Arc::new(Self {
            source,
            plans,
            calls: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn calls(&self) -> Vec<ScriptCall> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

fn reply_element(value: &CacheCounterValue) -> ScriptReply {
    match value {
        CacheCounterValue::Int(number) => ScriptReply::Int(*number),
        CacheCounterValue::Float(number) => ScriptReply::Float(*number),
        CacheCounterValue::Text(text) => ScriptReply::Text(text.clone()),
        CacheCounterValue::Bytes(bytes) => ScriptReply::Bytes(bytes.clone()),
    }
}

#[async_trait::async_trait]
impl CounterScript for PlannedScript {
    fn source(&self) -> &str {
        self.source
    }

    async fn run(
        &self,
        keys: &[String],
        args: &[RedisScalar],
    ) -> Result<ScriptReply, RedisCountersError> {
        let index = {
            let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
            calls.push((keys.to_vec(), args.to_vec()));
            calls.len().saturating_sub(1)
        };
        let Some(plan) = self.plans.get(index) else {
            return Err(RedisCountersError::UnexpectedReply {
                operation: "planned gauge script",
            });
        };
        match plan {
            ReplyPlan::Values(values) => Ok(ScriptReply::Array(
                values.iter().map(reply_element).collect(),
            )),
            ReplyPlan::Element(value) => Ok(reply_element(value)),
            ReplyPlan::Failure => Err(RedisCountersError::UnexpectedReply {
                operation: "planned gauge script",
            }),
        }
    }
}

pub(crate) struct ReleaseRegistration {
    pub(crate) source: String,
    pub(crate) keys: Vec<String>,
    pub(crate) args: Vec<RedisScalar>,
    pub(crate) hook: Arc<dyn ReleaseSettledHook>,
}

pub(crate) struct RecordingReleaseBatch {
    registrations: Mutex<Vec<ReleaseRegistration>>,
}

impl RecordingReleaseBatch {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            registrations: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn registrations(&self) -> Vec<ReleaseRegistration> {
        let registrations = self
            .registrations
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        registrations
            .iter()
            .map(|registration| ReleaseRegistration {
                source: registration.source.clone(),
                keys: registration.keys.clone(),
                args: registration.args.clone(),
                hook: Arc::clone(&registration.hook),
            })
            .collect()
    }
}

impl ReleaseBatch for RecordingReleaseBatch {
    fn register_release(
        &self,
        script: Arc<dyn CounterScript>,
        keys: &[String],
        args: &[RedisScalar],
        hook: Arc<dyn ReleaseSettledHook>,
    ) {
        self.registrations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(ReleaseRegistration {
                source: script.source().to_owned(),
                keys: keys.to_vec(),
                args: args.to_vec(),
                hook,
            });
    }
}

pub(crate) struct FixedBatchSource {
    pub(crate) batch: Option<Arc<RecordingReleaseBatch>>,
}

impl ReleaseBatchSource for FixedBatchSource {
    fn active_release_batch(&self) -> Option<Arc<dyn ReleaseBatch>> {
        let batch = self.batch.as_ref()?;
        let shared: Arc<dyn ReleaseBatch> = Arc::<RecordingReleaseBatch>::clone(batch);
        Some(shared)
    }
}

struct Captured(Arc<Mutex<Vec<u8>>>);

impl io::Write for Captured {
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

pub(crate) struct LogCapture {
    buffer: Arc<Mutex<Vec<u8>>>,
    _guard: DefaultGuard,
}

impl LogCapture {
    pub(crate) fn start() -> Self {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&buffer);
        let subscriber = fmt()
            .without_time()
            .with_ansi(false)
            .with_max_level(Level::TRACE)
            .with_writer(move || Captured(Arc::clone(&sink)))
            .finish();
        Self {
            buffer,
            _guard: set_default(subscriber),
        }
    }

    pub(crate) fn text(&self) -> String {
        String::from_utf8(self.buffer.lock().expect("the capture lock").clone())
            .expect("the capture is UTF-8")
    }
}

pub(crate) fn batch_failure() -> BatchError {
    BatchError::Cancelled
}
