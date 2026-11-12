use std::future::pending;
use std::io::{self, Write};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use betterlite_cache::backend::memory::MemoryCacheOps;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::backend::{BackendError, CacheResult};
use betterlite_cache::dual_cache::{
    CacheOptions, DualCache, DualCacheError, DualCacheSettings, MAX_REDIS_BATCH_CACHE_SIZE,
    MemoryMaxSize,
};
use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_models::decisions::Json;
use betterlite_redis::batch::result::BatchResult;
use betterlite_redis::redis_cache::RedisCache;
use tracing::Subscriber;
use tracing::subscriber::with_default;

use crate::auth_cache::backend::{AuthCacheTier, PreCall};
use crate::auth_cache::gateway_cache;
use crate::auth_cache::{AuthCache, AuthCacheError};

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn backend_failure(label: &'static str) -> DualCacheError {
    DualCacheError::Memory(BackendError::Unsupported(label))
}

pub(crate) fn assert_backend_failure(result: Result<(), AuthCacheError>, label: &'static str) {
    match result {
        Err(AuthCacheError::Backend(error)) => assert_eq!(failure_label(&error), Some(label)),
        other => panic!("expected a backend failure, got {other:?}"),
    }
}

pub(crate) fn failure_label(error: &DualCacheError) -> Option<&'static str> {
    match error {
        DualCacheError::Memory(BackendError::Unsupported(label)) => Some(label),
        DualCacheError::Memory(_)
        | DualCacheError::RedisIncrement(_)
        | DualCacheError::RedisTtl(_)
        | DualCacheError::RedisSetAdd(_)
        | DualCacheError::RedisDelete(_)
        | DualCacheError::FractionalIncrement { .. }
        | DualCacheError::CountOutOfRange { .. }
        | DualCacheError::UnrepresentableValue
        | DualCacheError::NotASet { .. } => None,
    }
}

#[derive(Default)]
pub(crate) struct State {
    pub(crate) ttl: Option<f64>,
    pub(crate) pre_call: Option<PreCall>,
    pub(crate) delete_failure: Option<DualCacheError>,
    pub(crate) delete_keys_failure: Option<DualCacheError>,
    pub(crate) pipeline_failure: Option<DualCacheError>,
    pub(crate) sets: Vec<(Option<String>, Json)>,
    pub(crate) pipelines: Vec<Vec<(String, Json)>>,
    pub(crate) delete_batches: Vec<Vec<String>>,
    pub(crate) pre_calls: Vec<Option<f64>>,
    pub(crate) reads: Vec<(String, bool)>,
    pub(crate) options: Vec<CacheOptions>,
    pub(crate) max_sizes: Vec<Option<usize>>,
    pub(crate) attached_ttls: Vec<Option<f64>>,
    pub(crate) deleted: Vec<String>,
    pub(crate) hang_delete_keys: bool,
}

pub(crate) struct RecordingMemory;

impl MemoryCacheOps for RecordingMemory {
    fn default_ttl(&self) -> i64 {
        60
    }

    fn get_cache(&self, _key: &str) -> CacheResult<Option<CacheValue>> {
        Ok(None)
    }

    fn set_cache(&self, _key: &str, _value: CacheValue, _ttl: Option<f64>) -> CacheResult<()> {
        Ok(())
    }

    fn delete_cache(&self, _key: &str) -> CacheResult<()> {
        Ok(())
    }

    fn flush_cache(&self) -> CacheResult<()> {
        Ok(())
    }

    fn batch_get_cache(&self, keys: &[String]) -> CacheResult<Vec<Option<CacheValue>>> {
        Ok(keys.iter().map(|_key| None).collect())
    }

    fn increment_cache(&self, _key: &str, _value: f64, _ttl: Option<f64>) -> CacheResult<f64> {
        Ok(0.0_f64)
    }

    fn ttl_entries(&self) -> CacheResult<Vec<(String, f64)>> {
        Ok(Vec::new())
    }
}

impl MemoryMaxSize for RecordingMemory {
    fn set_max_size(&self, _max_size: usize) {}
}

pub(crate) struct RecordingTier {
    pub(crate) name: &'static str,
    pub(crate) memory: Arc<RecordingMemory>,
    pub(crate) state: Arc<Mutex<State>>,
    pub(crate) log: Arc<Mutex<Vec<String>>>,
}

impl RecordingTier {
    fn new(name: &'static str, log: Arc<Mutex<Vec<String>>>, state: Arc<Mutex<State>>) -> Self {
        Self {
            name,
            memory: Arc::new(RecordingMemory),
            state,
            log,
        }
    }

    fn record(&self, operation: &str) {
        lock(&self.log).push(format!("{}:{operation}", self.name));
    }
}

#[async_trait]
impl AuthCacheTier for RecordingTier {
    type Memory = RecordingMemory;

    fn in_memory(&self) -> &Arc<RecordingMemory> {
        &self.memory
    }

    fn memory_ttl(&self) -> Option<f64> {
        lock(&self.state).ttl
    }

    fn update_cache_ttl(&self, memory_ttl: Option<f64>, redis_ttl: Option<f64>) {
        self.record("update_ttl");
        let mut state = lock(&self.state);
        if let Some(ttl) = memory_ttl {
            state.ttl = Some(ttl);
        }
        state.attached_ttls.push(redis_ttl);
    }

    fn update_in_memory_max_size(&self, max_size: Option<usize>) {
        self.record("update_max_size");
        lock(&self.state).max_sizes.push(max_size);
    }

    fn attach_redis_cache(&self, redis: Option<Arc<RedisCache>>, redis_ttl: Option<f64>) {
        self.record("attach_redis");
        drop(redis);
        lock(&self.state).attached_ttls.push(redis_ttl);
    }

    fn get(&self, key: &str, local_only: bool) -> Option<Json> {
        self.record("get");
        lock(&self.state).reads.push((key.to_owned(), local_only));
        None
    }

    async fn async_get(&self, key: &str, local_only: bool) -> Option<Json> {
        self.get(key, local_only)
    }

    fn set(&self, key: Option<&str>, value: &Json, options: CacheOptions) {
        self.record("set");
        let mut state = lock(&self.state);
        state.sets.push((key.map(str::to_owned), value.clone()));
        state.options.push(options);
    }

    async fn async_set(&self, key: Option<&str>, value: &Json, options: CacheOptions) {
        self.record("async_set");
        let mut state = lock(&self.state);
        state.sets.push((key.map(str::to_owned), value.clone()));
        state.options.push(options);
    }

    fn set_pre_call(
        &self,
        key: &str,
        value: &Json,
        ttl: Option<f64>,
    ) -> Result<PreCall, DualCacheError> {
        self.record("set_pre_call");
        let mut state = lock(&self.state);
        state.pre_calls.push(ttl);
        state.sets.push((Some(key.to_owned()), value.clone()));
        Ok(state.pre_call.unwrap_or(PreCall::Direct))
    }

    fn delete(&self, key: &str) -> Result<(), DualCacheError> {
        self.record("delete");
        let mut state = lock(&self.state);
        state.deleted.push(key.to_owned());
        state.delete_failure.take().map_or(Ok(()), Err)
    }

    async fn async_delete(&self, key: &str) -> Result<(), DualCacheError> {
        self.delete(key)
    }

    fn delete_pre_call(&self, key: &str) -> Result<Option<BatchResult<()>>, DualCacheError> {
        self.record("delete_pre_call");
        lock(&self.state).deleted.push(key.to_owned());
        Ok(None)
    }

    async fn async_delete_keys(&self, keys: &[String]) -> Result<(), DualCacheError> {
        self.record("delete_keys");
        let (hanging, failure) = {
            let mut state = lock(&self.state);
            state.delete_batches.push(keys.to_vec());
            (state.hang_delete_keys, state.delete_keys_failure.take())
        };
        if hanging {
            pending::<()>().await;
        }
        failure.map_or(Ok(()), Err)
    }

    async fn async_set_pipeline(
        &self,
        entries: &[(String, Json)],
        options: CacheOptions,
    ) -> Result<(), DualCacheError> {
        self.record("set_pipeline");
        let mut state = lock(&self.state);
        state.pipelines.push(entries.to_vec());
        state.options.push(options);
        state.pipeline_failure.take().map_or(Ok(()), Err)
    }
}

pub(crate) struct RecordingCache {
    pub(crate) cache: AuthCache<RecordingTier>,
    pub(crate) log: Arc<Mutex<Vec<String>>>,
    pub(crate) ordinary: Arc<Mutex<State>>,
    pub(crate) key_objects: Arc<Mutex<State>>,
}

impl RecordingCache {
    pub(crate) fn operations(&self) -> Vec<String> {
        lock(&self.log).clone()
    }
}

pub(crate) fn recording_cache() -> RecordingCache {
    let log = Arc::new(Mutex::new(Vec::new()));
    let ordinary = Arc::new(Mutex::new(State::default()));
    let key_objects = Arc::new(Mutex::new(State::default()));
    RecordingCache {
        cache: AuthCache {
            ordinary: RecordingTier::new("ordinary", Arc::clone(&log), Arc::clone(&ordinary)),
            key_objects: RecordingTier::new("key", Arc::clone(&log), Arc::clone(&key_objects)),
            management_ttl_fallback: 45.0_f64,
        },
        log,
        ordinary,
        key_objects,
    }
}

fn clock() -> Arc<dyn Fn() -> f64 + Send + Sync> {
    Arc::new(|| 1000.0_f64)
}

pub(crate) fn memory() -> Arc<LocalMemoryOps> {
    Arc::new(LocalMemoryOps::new(1024, clock()).expect("the local memory tier builds"))
}

pub(crate) fn settings(memory_ttl: Option<f64>) -> DualCacheSettings {
    DualCacheSettings {
        default_in_memory_ttl: memory_ttl,
        default_redis_ttl: None,
        batch_cache_expiry: None,
        max_redis_batch_cache_size: MAX_REDIS_BATCH_CACHE_SIZE,
        redis_delete_chunk_size: MAX_REDIS_BATCH_CACHE_SIZE,
    }
}

pub(crate) type LocalCache = AuthCache<DualCache<LocalMemoryOps>>;

pub(crate) fn local_cache(memory_ttl: Option<f64>) -> LocalCache {
    gateway_cache(memory(), memory(), None, settings(memory_ttl), 45.0_f64)
}

pub(crate) fn redis_handle(cache: &LocalCache, key_objects: bool) -> Arc<RedisCache> {
    let tier = if key_objects {
        &cache.key_objects
    } else {
        &cache.ordinary
    };
    let held = tier.redis_cache.read().unwrap();
    held.clone().expect("the partition holds Redis")
}

#[derive(Clone)]
struct LogWriter(Arc<Mutex<Vec<u8>>>);

impl Write for LogWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        lock(&self.0).extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn capturing() -> (impl Subscriber + Send + Sync, Arc<Mutex<Vec<u8>>>) {
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let writer = LogWriter(Arc::clone(&bytes));
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .without_time()
        .with_ansi(false)
        .with_target(false)
        .with_writer(move || writer.clone())
        .finish();
    (subscriber, bytes)
}

pub(crate) fn capture<T, F>(operation: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let (subscriber, bytes) = capturing();
    let output = with_default(subscriber, operation);
    let text = String::from_utf8(lock(&bytes).clone()).unwrap();
    (output, text)
}
