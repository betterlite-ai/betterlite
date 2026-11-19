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
use crate::concurrency_gauges::cache::{GaugeCache, GaugeCacheValue};
use crate::fail_closed::FailClosedPolicy;
use crate::limiter_records::{
    CacheCounterValue, LimiterClock, RateLimitDescriptor, RateLimitDescriptorRateLimit,
    ScriptReply, WindowKeyMetadata,
};
use crate::rate_limit_window_store::{
    CounterCache, CounterScript, CounterValues, RedisCacheKind, RequestBatchSource, ScriptBatch,
    ScriptHandle, WindowStore,
};
use crate::windowed_admission::{AdmissionRequest, SlotIdGenerator, WindowedAdmission};

pub(crate) const NOW: i64 = 1_700_000_000;
pub(crate) const HANDLER_WINDOW_SIZE: i64 = 45;
pub(crate) const GENERATED_SLOT: &str = "generated-slot";

pub(crate) fn key_list(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

pub(crate) fn limits(
    requests: Option<i64>,
    tokens: Option<i64>,
    max_parallel: Option<i64>,
    window_size: Option<i64>,
) -> RateLimitDescriptorRateLimit {
    RateLimitDescriptorRateLimit {
        requests_per_unit: requests,
        tokens_per_unit: tokens,
        max_parallel_requests: max_parallel,
        window_size,
    }
}

pub(crate) fn descriptor(
    key: &str,
    value: &str,
    rate_limit: Option<RateLimitDescriptorRateLimit>,
) -> RateLimitDescriptor {
    RateLimitDescriptor {
        key: key.to_owned(),
        value: value.to_owned(),
        rate_limit,
    }
}

pub(crate) fn metadata(
    requests_limit: Option<i64>,
    tokens_limit: Option<i64>,
    window_size: i64,
) -> WindowKeyMetadata {
    WindowKeyMetadata {
        requests_limit,
        tokens_limit,
        window_size,
        descriptor_key: "api-key".to_owned(),
        descriptor_value: "k1".to_owned(),
    }
}

pub(crate) fn metadata_map(
    entries: &[(&str, WindowKeyMetadata)],
) -> BTreeMap<String, WindowKeyMetadata> {
    entries
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

pub(crate) fn int(value: i64) -> Option<CacheCounterValue> {
    Some(CacheCounterValue::Int(value))
}

pub(crate) fn text(value: &str) -> Option<CacheCounterValue> {
    Some(CacheCounterValue::Text(value.to_owned()))
}

pub(crate) fn request(descriptors: Vec<RateLimitDescriptor>) -> AdmissionRequest {
    AdmissionRequest {
        descriptors,
        parent_otel_span: None,
        read_only: false,
        skip_tpm_check: false,
        parallel_slot_id: None,
    }
}

pub(crate) struct FixedClock {
    at: SystemTime,
}

impl FixedClock {
    pub(crate) fn at(seconds: i64) -> Arc<Self> {
        let seconds = u64::try_from(seconds).expect("a positive clock time");
        let at = UNIX_EPOCH
            .checked_add(Duration::from_secs(seconds))
            .expect("a representable clock time");
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
    pub(crate) value: CacheCounterValue,
    pub(crate) ttl: i64,
    pub(crate) local_only: bool,
}

pub(crate) struct RecordingCache {
    stored: Mutex<BTreeMap<String, CacheCounterValue>>,
    reads: Mutex<Vec<(String, bool)>>,
    batch_reads: Mutex<Vec<(Vec<String>, bool)>>,
    writes: Mutex<Vec<CacheWrite>>,
    batch_fails: AtomicBool,
    cluster: AtomicBool,
}

impl RecordingCache {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            stored: Mutex::new(BTreeMap::new()),
            reads: Mutex::new(Vec::new()),
            batch_reads: Mutex::new(Vec::new()),
            writes: Mutex::new(Vec::new()),
            batch_fails: AtomicBool::new(false),
            cluster: AtomicBool::new(false),
        })
    }

    pub(crate) fn seed(&self, key: &str, value: CacheCounterValue) {
        self.stored
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value);
    }

    pub(crate) fn fail_batch_reads(&self) {
        self.batch_fails.store(true, Ordering::SeqCst);
    }

    pub(crate) fn as_cluster(&self) {
        self.cluster.store(true, Ordering::SeqCst);
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
impl CounterCache for RecordingCache {
    async fn get_cache(&self, key: &str, local_only: bool) -> Option<CacheCounterValue> {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((key.to_owned(), local_only));
        self.stored
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned()
    }

    async fn set_cache(&self, key: &str, value: CacheCounterValue, ttl: i64, local_only: bool) {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(CacheWrite {
                key: key.to_owned(),
                value: value.clone(),
                ttl,
                local_only,
            });
        self.stored
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value);
    }

    async fn batch_get_cache(&self, keys: &[String], local_only: bool) -> Option<CounterValues> {
        self.batch_reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((keys.to_vec(), local_only));
        if self.batch_fails.load(Ordering::SeqCst) {
            return None;
        }
        let stored = self.stored.lock().unwrap_or_else(PoisonError::into_inner);
        Some(keys.iter().map(|key| stored.get(key).cloned()).collect())
    }

    fn redis_cache_kind(&self) -> Option<RedisCacheKind> {
        if self.cluster.load(Ordering::SeqCst) {
            return Some(RedisCacheKind::Cluster);
        }
        Some(RedisCacheKind::Standalone)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ReplyPlan {
    Values(Vec<CacheCounterValue>),
    Failure,
}

pub(crate) fn int_plan(values: &[i64]) -> ReplyPlan {
    ReplyPlan::Values(values.iter().copied().map(CacheCounterValue::Int).collect())
}

pub(crate) fn mixed_plan(values: &[Option<CacheCounterValue>]) -> ReplyPlan {
    let planned = values
        .iter()
        .map(|value| value.clone().expect("a planned script value"))
        .collect();
    ReplyPlan::Values(planned)
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ScriptCall {
    pub(crate) keys: Vec<String>,
    pub(crate) args: Vec<RedisScalar>,
}

fn reply_element(value: &CacheCounterValue) -> ScriptReply {
    match value {
        CacheCounterValue::Int(number) => ScriptReply::Int(*number),
        CacheCounterValue::Float(number) => ScriptReply::Float(*number),
        CacheCounterValue::Text(text) => ScriptReply::Text(text.clone()),
        CacheCounterValue::Bytes(bytes) => ScriptReply::Bytes(bytes.clone()),
    }
}

fn planned_reply(plan: Option<&ReplyPlan>) -> Result<ScriptReply, RedisCountersError> {
    match plan {
        Some(ReplyPlan::Values(values)) => Ok(ScriptReply::Array(
            values.iter().map(reply_element).collect(),
        )),
        Some(ReplyPlan::Failure) | None => Err(RedisCountersError::UnexpectedReply {
            operation: "planned rate limiter script",
        }),
    }
}

fn record_call(calls: &Mutex<Vec<ScriptCall>>, keys: &[String], args: &[RedisScalar]) -> usize {
    let mut calls = calls.lock().unwrap_or_else(PoisonError::into_inner);
    calls.push(ScriptCall {
        keys: keys.to_vec(),
        args: args.to_vec(),
    });
    calls.len().saturating_sub(1)
}

pub(crate) struct PlannedScript {
    plans: Vec<ReplyPlan>,
    calls: Mutex<Vec<ScriptCall>>,
}

impl PlannedScript {
    pub(crate) fn new(plans: Vec<ReplyPlan>) -> Arc<Self> {
        Arc::new(Self {
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

#[async_trait::async_trait]
impl CounterScript for PlannedScript {
    fn source(&self) -> &str {
        "planned rate limiter script"
    }

    async fn run(
        &self,
        keys: &[String],
        args: &[RedisScalar],
    ) -> Result<ScriptReply, RedisCountersError> {
        let index = record_call(&self.calls, keys, args);
        planned_reply(self.plans.get(index))
    }
}

pub(crate) struct PlannedBatch {
    plans: Vec<ReplyPlan>,
    calls: Mutex<Vec<ScriptCall>>,
}

impl PlannedBatch {
    pub(crate) fn new(plans: Vec<ReplyPlan>) -> Arc<Self> {
        Arc::new(Self {
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

impl ScriptBatch for PlannedBatch {
    fn script(
        &self,
        _script: Arc<dyn CounterScript>,
        keys: &[String],
        args: &[RedisScalar],
    ) -> ScriptHandle {
        let index = record_call(&self.calls, keys, args);
        let reply =
            planned_reply(self.plans.get(index)).map_err(|_error| Arc::new(BatchError::Cancelled));
        Box::pin(async move { reply })
    }
}

pub(crate) struct BatchSource {
    pub(crate) batch: Arc<PlannedBatch>,
}

impl RequestBatchSource for BatchSource {
    fn active_batch(&self) -> Option<Arc<dyn ScriptBatch>> {
        Some(Arc::clone(&self.batch))
    }
}

pub(crate) struct SlotIds {
    generated: String,
}

impl SlotIds {
    pub(crate) fn new(generated: &str) -> Arc<Self> {
        Arc::new(Self {
            generated: generated.to_owned(),
        })
    }
}

impl SlotIdGenerator for SlotIds {
    fn next_slot_id(&self) -> String {
        self.generated.clone()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GaugeWrite {
    pub(crate) key: String,
    pub(crate) value: GaugeCacheValue,
    pub(crate) ttl: Duration,
    pub(crate) local_only: bool,
}

pub(crate) struct RecordingGauges {
    stored: Mutex<BTreeMap<String, GaugeCacheValue>>,
    reads: Mutex<Vec<(String, bool)>>,
    batch_reads: Mutex<Vec<(Vec<String>, bool)>>,
    writes: Mutex<Vec<GaugeWrite>>,
}

impl RecordingGauges {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            stored: Mutex::new(BTreeMap::new()),
            reads: Mutex::new(Vec::new()),
            batch_reads: Mutex::new(Vec::new()),
            writes: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn seed(&self, key: &str, value: GaugeCacheValue) {
        self.stored
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value);
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

    pub(crate) fn writes(&self) -> Vec<GaugeWrite> {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait::async_trait]
impl GaugeCache for RecordingGauges {
    async fn get(&self, key: &str, local_only: bool) -> Option<GaugeCacheValue> {
        self.reads
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((key.to_owned(), local_only));
        self.stored
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned()
    }

    async fn set(&self, key: &str, value: GaugeCacheValue, ttl: Duration, local_only: bool) {
        self.writes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(GaugeWrite {
                key: key.to_owned(),
                value: value.clone(),
                ttl,
                local_only,
            });
        self.stored
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
        let stored = self.stored.lock().unwrap_or_else(PoisonError::into_inner);
        Some(keys.iter().map(|key| stored.get(key).cloned()).collect())
    }
}

pub(crate) struct Harness {
    pub(crate) cache: Arc<RecordingCache>,
    pub(crate) script: Arc<PlannedScript>,
    pub(crate) refunds: Arc<PlannedScript>,
    pub(crate) batch: Arc<PlannedBatch>,
    pub(crate) gauges: Arc<RecordingGauges>,
    pub(crate) slots: Arc<SlotIds>,
    pub(crate) clock: Arc<FixedClock>,
    pub(crate) fail_closed: bool,
    pub(crate) window_size: i64,
    pub(crate) script_enabled: bool,
    pub(crate) batch_enabled: bool,
}

pub(crate) fn harness() -> Harness {
    Harness {
        cache: RecordingCache::new(),
        script: PlannedScript::new(Vec::new()),
        refunds: PlannedScript::new(Vec::new()),
        batch: PlannedBatch::new(Vec::new()),
        gauges: RecordingGauges::new(),
        slots: SlotIds::new(GENERATED_SLOT),
        clock: FixedClock::at(NOW),
        fail_closed: false,
        window_size: HANDLER_WINDOW_SIZE,
        script_enabled: true,
        batch_enabled: false,
    }
}

impl Harness {
    pub(crate) fn limiter(&self) -> WindowedAdmission {
        let script: Option<Arc<dyn CounterScript>> = if self.script_enabled {
            Some(Arc::clone(&self.script))
        } else {
            None
        };
        let request_batch: Option<Arc<dyn RequestBatchSource>> = if self.batch_enabled {
            Some(Arc::new(BatchSource {
                batch: Arc::clone(&self.batch),
            }))
        } else {
            None
        };
        let fail_closed = FailClosedPolicy {
            enabled: self.fail_closed,
        };
        let gauges = ConcurrencyGauges {
            cache: Arc::clone(&self.gauges),
            clock: Arc::clone(&self.clock),
            acquire_script: None,
            release_script: None,
            count_script: None,
            release_batch: None,
            fail_closed,
            check_and_increment_lock: AsyncMutex::new(()),
        };
        WindowedAdmission {
            window_size: self.window_size,
            clock: Arc::clone(&self.clock),
            fail_closed,
            windows: Arc::new(WindowStore {
                cache: Arc::clone(&self.cache),
                rate_limiter_script: script,
                counter_read_script: None,
                request_batch,
                check_and_increment_lock: AsyncMutex::new(()),
            }),
            refund_script: Some(Arc::clone(&self.refunds)),
            gauges: Arc::new(gauges),
            slot_ids: Arc::clone(&self.slots),
        }
    }
}

pub(crate) fn gauge_key() -> String {
    "{api-key:k1}:max_parallel_requests".to_owned()
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
            .with_max_level(Level::WARN)
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
