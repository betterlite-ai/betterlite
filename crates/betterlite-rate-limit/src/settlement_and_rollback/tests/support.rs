use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_redis::batch::error::BatchError;
use betterlite_redis::cache_primitives::protocol::RedisScalar;
use betterlite_redis::redis_counters::RedisCountersError;
use tokio::sync::Mutex as AsyncMutex;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt;

use crate::atomic_counter::payload::IncrementAmounts;
use crate::fail_closed::FailClosedPolicy;
use crate::limiter_records::{
    AtomicCounterMeta, CacheCounterValue, LimiterClock, RateLimitDescriptor,
    RateLimitDescriptorRateLimit, ScriptReply, WindowRateLimitType,
};
use crate::rate_limit_window_store::{
    CounterCache, CounterScript, CounterValues, RedisCacheKind, RequestBatchSource, ScriptBatch,
    ScriptHandle, WindowStore,
};
use crate::settlement_and_rollback::AtomicSettlement;

pub(crate) const WINDOW: i64 = 60;

pub(crate) fn descriptor(
    key: &str,
    value: &str,
    requests: Option<i64>,
    tokens: Option<i64>,
) -> RateLimitDescriptor {
    RateLimitDescriptor {
        key: key.to_owned(),
        value: value.to_owned(),
        rate_limit: Some(RateLimitDescriptorRateLimit {
            requests_per_unit: requests,
            tokens_per_unit: tokens,
            max_parallel_requests: None,
            window_size: None,
        }),
    }
}

pub(crate) fn unlimited(key: &str, value: &str) -> RateLimitDescriptor {
    RateLimitDescriptor {
        key: key.to_owned(),
        value: value.to_owned(),
        rate_limit: None,
    }
}

pub(crate) const fn amounts(requests: i64, tokens: i64) -> IncrementAmounts {
    IncrementAmounts { requests, tokens }
}

pub(crate) fn reply(values: &[i64]) -> ScriptReply {
    ScriptReply::Array(
        values
            .iter()
            .map(|value| ScriptReply::Int(*value))
            .collect(),
    )
}

pub(crate) fn empty_reply() -> ScriptReply {
    ScriptReply::Array(Vec::new())
}

pub(crate) fn key_list(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

pub(crate) fn text(value: &str) -> RedisScalar {
    RedisScalar::Text(value.to_owned())
}

pub(crate) fn int_value(value: i64) -> CacheCounterValue {
    CacheCounterValue::Int(value)
}

pub(crate) fn text_value(value: &str) -> CacheCounterValue {
    CacheCounterValue::Text(value.to_owned())
}

pub(crate) fn meta(window_key: &str, counter_key: &str, increment: i64) -> AtomicCounterMeta {
    AtomicCounterMeta {
        descriptor_key: "model".to_owned(),
        descriptor_value: "gpt-4".to_owned(),
        current_limit: 10,
        rate_limit_type: WindowRateLimitType::Requests,
        window_key: window_key.to_owned(),
        counter_key: counter_key.to_owned(),
        increment,
        ttl: WINDOW,
        window_size: WINDOW,
    }
}

pub(crate) struct FixedClock {
    pub(crate) at: SystemTime,
}

impl FixedClock {
    pub(crate) fn at(seconds: u64) -> Self {
        Self {
            at: UNIX_EPOCH + Duration::from_secs(seconds),
        }
    }

    pub(crate) fn before_epoch() -> Self {
        Self {
            at: UNIX_EPOCH
                .checked_sub(Duration::from_secs(1))
                .expect("the Unix epoch has a second before it"),
        }
    }
}

impl LimiterClock for FixedClock {
    fn current_time(&self) -> SystemTime {
        self.at
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CacheRead {
    pub(crate) key: String,
    pub(crate) local_only: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CacheWrite {
    pub(crate) key: String,
    pub(crate) value: CacheCounterValue,
    pub(crate) ttl: i64,
    pub(crate) local_only: bool,
}

pub(crate) struct FakeCache {
    stored: Mutex<BTreeMap<String, CacheCounterValue>>,
    reads: Mutex<Vec<CacheRead>>,
    writes: Mutex<Vec<CacheWrite>>,
}

impl FakeCache {
    pub(crate) fn empty() -> Arc<Self> {
        Arc::new(Self {
            stored: Mutex::new(BTreeMap::new()),
            reads: Mutex::new(Vec::new()),
            writes: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn seeded(entries: &[(&str, CacheCounterValue)]) -> Arc<Self> {
        let cache = Self::empty();
        for (key, value) in entries {
            cache
                .stored
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert((*key).to_owned(), value.clone());
        }
        cache
    }

    pub(crate) fn reads(&self) -> Vec<CacheRead> {
        self.reads
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
impl CounterCache for FakeCache {
    async fn get_cache(&self, key: &str, local_only: bool) -> Option<CacheCounterValue> {
        let mut reads = self.reads.lock().unwrap_or_else(PoisonError::into_inner);
        reads.push(CacheRead {
            key: key.to_owned(),
            local_only,
        });
        drop(reads);
        self.stored
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned()
    }

    async fn set_cache(&self, key: &str, value: CacheCounterValue, ttl: i64, local_only: bool) {
        let mut writes = self.writes.lock().unwrap_or_else(PoisonError::into_inner);
        writes.push(CacheWrite {
            key: key.to_owned(),
            value: value.clone(),
            ttl,
            local_only,
        });
        drop(writes);
        self.stored
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.to_owned(), value);
    }

    async fn batch_get_cache(&self, _keys: &[String], _local_only: bool) -> Option<CounterValues> {
        None
    }

    fn redis_cache_kind(&self) -> Option<RedisCacheKind> {
        None
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ScriptRun {
    pub(crate) keys: Vec<String>,
    pub(crate) args: Vec<RedisScalar>,
}

pub(crate) struct FakeScript {
    replies: Vec<ScriptReply>,
    fail_at: Vec<usize>,
    calls: Mutex<Vec<ScriptRun>>,
}

impl FakeScript {
    pub(crate) fn new(replies: Vec<ScriptReply>) -> Arc<Self> {
        Arc::new(Self {
            replies,
            fail_at: Vec::new(),
            calls: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn failing(replies: Vec<ScriptReply>, fail_at: &[usize]) -> Arc<Self> {
        Arc::new(Self {
            replies,
            fail_at: fail_at.to_vec(),
            calls: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn calls(&self) -> Vec<ScriptRun> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait::async_trait]
impl CounterScript for FakeScript {
    fn source(&self) -> &str {
        "fixture script"
    }

    async fn run(
        &self,
        keys: &[String],
        args: &[RedisScalar],
    ) -> Result<ScriptReply, RedisCountersError> {
        let call = {
            let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
            calls.push(ScriptRun {
                keys: keys.to_vec(),
                args: args.to_vec(),
            });
            calls.len().saturating_sub(1)
        };
        if self.fail_at.contains(&call) {
            return Err(RedisCountersError::UnexpectedReply {
                operation: "fixture script",
            });
        }
        self.replies
            .get(call)
            .cloned()
            .ok_or(RedisCountersError::UnexpectedReply {
                operation: "no fixture reply",
            })
    }
}

pub(crate) struct FakeBatch {
    replies: Vec<ScriptReply>,
    fail_at: Vec<usize>,
    calls: Mutex<Vec<ScriptRun>>,
}

impl FakeBatch {
    pub(crate) fn new(replies: Vec<ScriptReply>, fail_at: &[usize]) -> Arc<Self> {
        Arc::new(Self {
            replies,
            fail_at: fail_at.to_vec(),
            calls: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn calls(&self) -> Vec<ScriptRun> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ScriptBatch for FakeBatch {
    fn script(
        &self,
        _script: Arc<dyn CounterScript>,
        keys: &[String],
        args: &[RedisScalar],
    ) -> ScriptHandle {
        let call = {
            let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
            calls.push(ScriptRun {
                keys: keys.to_vec(),
                args: args.to_vec(),
            });
            calls.len().saturating_sub(1)
        };
        let reply = self.replies.get(call).cloned();
        let failure = self.fail_at.contains(&call);
        Box::pin(async move {
            if failure {
                return Err(Arc::new(BatchError::Cancelled));
            }
            reply.ok_or_else(|| Arc::new(BatchError::Cancelled))
        })
    }
}

pub(crate) struct FakeBatchSource {
    pub(crate) batch: Arc<FakeBatch>,
}

impl RequestBatchSource for FakeBatchSource {
    fn active_batch(&self) -> Option<Arc<dyn ScriptBatch>> {
        let shared: Arc<dyn ScriptBatch> = Arc::<FakeBatch>::clone(&self.batch);
        Some(shared)
    }
}

pub(crate) fn store(cache: Arc<FakeCache>) -> WindowStore {
    WindowStore {
        cache,
        rate_limiter_script: None,
        counter_read_script: None,
        request_batch: None,
        check_and_increment_lock: AsyncMutex::new(()),
    }
}

pub(crate) fn pipelined_store(cache: Arc<FakeCache>, batch: Arc<FakeBatch>) -> WindowStore {
    WindowStore {
        request_batch: Some(Arc::new(FakeBatchSource { batch })),
        ..store(cache)
    }
}

fn script_object<S>(script: Arc<S>) -> Arc<dyn CounterScript>
where
    S: CounterScript + 'static,
{
    script
}

pub(crate) fn settlement<'settlement>(
    windows: &'settlement WindowStore,
    clock: &'settlement FixedClock,
    script: Option<Arc<FakeScript>>,
    refund_script: Option<Arc<FakeScript>>,
    fail_closed: bool,
) -> AtomicSettlement<'settlement> {
    AtomicSettlement {
        windows,
        clock,
        script: script.map(script_object),
        refund_script: refund_script.map(script_object),
        fail_closed: FailClosedPolicy {
            enabled: fail_closed,
        },
        default_window_size: WINDOW,
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
        String::from_utf8(
            self.buffer
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone(),
        )
        .expect("the capture is UTF-8")
    }
}
