use std::collections::BTreeMap;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_redis::batch::error::BatchError;
use betterlite_redis::cache_primitives::protocol::RedisScalar;
use betterlite_redis::redis_counters::RedisCountersError;
use tokio::sync::Mutex as AsyncMutex;
use tracing::Level;
use tracing::subscriber::{DefaultGuard, set_default};
use tracing_subscriber::fmt;

use crate::limiter_records::{CacheCounterValue, ScriptReply};
use crate::rate_limit_window_store::{
    CounterCache, CounterScript, CounterValues, RedisCacheKind, RequestBatchSource, ScriptBatch,
    ScriptHandle, WindowStore,
};

pub(crate) fn key_list(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

pub(crate) fn store_with<C>(cache: Arc<C>) -> WindowStore
where
    C: CounterCache + 'static,
{
    WindowStore {
        cache,
        rate_limiter_script: None,
        counter_read_script: None,
        request_batch: None,
        check_and_increment_lock: AsyncMutex::new(()),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CacheWrite {
    pub(crate) key: String,
    pub(crate) value: CacheCounterValue,
    pub(crate) ttl: i64,
    pub(crate) local_only: bool,
}

pub(crate) struct FakeCache {
    kind: Option<RedisCacheKind>,
    stored: Mutex<BTreeMap<String, CacheCounterValue>>,
    gets: Mutex<Vec<(String, bool)>>,
    writes: Mutex<Vec<CacheWrite>>,
    batch_reads: Mutex<Vec<(Vec<String>, bool)>>,
    batch_reply: Mutex<Option<CounterValues>>,
    batch_fails: AtomicBool,
}

impl FakeCache {
    pub(crate) fn new(kind: Option<RedisCacheKind>) -> Arc<Self> {
        Arc::new(Self {
            kind,
            stored: Mutex::new(BTreeMap::new()),
            gets: Mutex::new(Vec::new()),
            writes: Mutex::new(Vec::new()),
            batch_reads: Mutex::new(Vec::new()),
            batch_reply: Mutex::new(None),
            batch_fails: AtomicBool::new(false),
        })
    }

    pub(crate) fn seed(&self, key: &str, value: CacheCounterValue) {
        self.stored
            .lock()
            .expect("the cache lock")
            .insert(key.to_owned(), value);
    }

    pub(crate) fn gets(&self) -> Vec<(String, bool)> {
        self.gets.lock().expect("the cache lock").clone()
    }

    pub(crate) fn writes(&self) -> Vec<CacheWrite> {
        self.writes.lock().expect("the cache lock").clone()
    }

    pub(crate) fn batch_reads(&self) -> Vec<(Vec<String>, bool)> {
        self.batch_reads.lock().expect("the cache lock").clone()
    }

    pub(crate) fn set_batch_reply(&self, values: CounterValues) {
        *self.batch_reply.lock().expect("the cache lock") = Some(values);
    }

    pub(crate) fn fail_batch_reads(&self) {
        self.batch_fails.store(true, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl CounterCache for FakeCache {
    async fn get_cache(&self, key: &str, local_only: bool) -> Option<CacheCounterValue> {
        self.gets
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
            .expect("the cache lock")
            .push(CacheWrite {
                key: key.to_owned(),
                value: value.clone(),
                ttl,
                local_only,
            });
        self.stored
            .lock()
            .expect("the cache lock")
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
        let canned = self
            .batch_reply
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(values) = canned {
            return Some(values);
        }
        let stored = self.stored.lock().unwrap_or_else(PoisonError::into_inner);
        Some(keys.iter().map(|key| stored.get(key).cloned()).collect())
    }

    fn redis_cache_kind(&self) -> Option<RedisCacheKind> {
        self.kind
    }
}

pub(crate) struct FakeScript {
    source: &'static str,
    replies: Vec<ScriptReply>,
    fail_at: Option<usize>,
    calls: Mutex<Vec<(Vec<String>, Vec<RedisScalar>)>>,
}

impl FakeScript {
    pub(crate) fn new(source: &'static str, replies: Vec<ScriptReply>) -> Arc<Self> {
        Arc::new(Self {
            source,
            replies,
            fail_at: None,
            calls: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn failing_after(self: &Arc<Self>, call: usize) -> Arc<Self> {
        Arc::new(Self {
            source: self.source,
            replies: self.replies.clone(),
            fail_at: Some(call),
            calls: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn calls(&self) -> Vec<(Vec<String>, Vec<RedisScalar>)> {
        self.calls.lock().expect("the script lock").clone()
    }
}

#[async_trait::async_trait]
impl CounterScript for FakeScript {
    fn source(&self) -> &str {
        self.source
    }

    async fn run(
        &self,
        keys: &[String],
        args: &[RedisScalar],
    ) -> Result<ScriptReply, RedisCountersError> {
        let mut calls = self.calls.lock().unwrap_or_else(PoisonError::into_inner);
        calls.push((keys.to_vec(), args.to_vec()));
        let call = calls.len();
        drop(calls);
        if self.fail_at == Some(call) {
            return Err(RedisCountersError::UnexpectedReply {
                operation: "test script",
            });
        }
        self.replies.get(call.saturating_sub(1)).cloned().ok_or(
            RedisCountersError::UnexpectedReply {
                operation: "no script reply left",
            },
        )
    }
}

pub(crate) struct FakeBatch {
    calls: Mutex<Vec<(Vec<String>, Vec<RedisScalar>)>>,
    reply: ScriptReply,
}

impl FakeBatch {
    pub(crate) fn new(reply: ScriptReply) -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            reply,
        })
    }

    pub(crate) fn calls(&self) -> Vec<(Vec<String>, Vec<RedisScalar>)> {
        self.calls.lock().expect("the batch lock").clone()
    }
}

impl ScriptBatch for FakeBatch {
    fn script(
        &self,
        _script: Arc<dyn CounterScript>,
        keys: &[String],
        args: &[RedisScalar],
    ) -> ScriptHandle {
        self.calls
            .lock()
            .expect("the batch lock")
            .push((keys.to_vec(), args.to_vec()));
        let reply = self.reply.clone();
        Box::pin(async move {
            let answer: Result<ScriptReply, Arc<BatchError>> = Ok(reply);
            answer
        })
    }
}

pub(crate) struct FakeBatchSource {
    pub(crate) batch: Option<Arc<FakeBatch>>,
}

impl RequestBatchSource for FakeBatchSource {
    fn active_batch(&self) -> Option<Arc<dyn ScriptBatch>> {
        let batch = self.batch.as_ref()?;
        let shared: Arc<dyn ScriptBatch> = Arc::<FakeBatch>::clone(batch);
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
