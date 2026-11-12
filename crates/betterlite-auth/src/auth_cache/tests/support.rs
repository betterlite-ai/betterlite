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

use crate::auth_cache::AuthCache;
use crate::auth_cache::backend::{AuthCacheTier, PreCall};
use crate::auth_cache::gateway_cache;

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn backend_failure(label: &'static str) -> DualCacheError {
    DualCacheError::Memory(BackendError::Unsupported(label))
}

pub(crate) fn failure_label(error: &DualCacheError) -> Option<&'static str> {
    match error {
        DualCacheError::Memory(BackendError::Unsupported(label)) => Some(label),
        DualCacheError::Memory(_) => None,
        DualCacheError::RedisIncrement(_)
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

    fn update_cache_ttl(&self, memory_ttl: Option<f64>, _redis_ttl: Option<f64>) {
        self.record("update_ttl");
        let mut state = lock(&self.state);
        if let Some(ttl) = memory_ttl {
            state.ttl = Some(ttl);
        }
    }

    fn update_in_memory_max_size(&self, _max_size: Option<usize>) {
        self.record("update_max_size");
    }

    fn attach_redis_cache(&self, _redis: Option<Arc<RedisCache>>, _redis_ttl: Option<f64>) {
        self.record("attach_redis");
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
        drop(key);
        match lock(&self.state).delete_failure.take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    async fn async_delete(&self, key: &str) -> Result<(), DualCacheError> {
        self.delete(key)
    }

    fn delete_pre_call(&self, key: &str) -> Result<Option<BatchResult<()>>, DualCacheError> {
        self.record("delete_pre_call");
        drop(key);
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
            std::future::pending::<()>().await;
        }
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    async fn async_set_pipeline(
        &self,
        entries: &[(String, Json)],
        _options: CacheOptions,
    ) -> Result<(), DualCacheError> {
        self.record("set_pipeline");
        let mut state = lock(&self.state);
        state.pipelines.push(entries.to_vec());
        match state.pipeline_failure.take() {
            Some(error) => Err(error),
            None => Ok(()),
        }
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

#[derive(Clone)]
struct LogWriter(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for LogWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        lock(&self.0).extend_from_slice(buffer);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn capturing() -> (impl tracing::Subscriber + Send + Sync, Arc<Mutex<Vec<u8>>>) {
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
    let output = tracing::subscriber::with_default(subscriber, operation);
    let text = String::from_utf8(lock(&bytes).clone()).unwrap();
    (output, text)
}

pub(crate) async fn capture_async<T, F>(future: F) -> (T, String)
where
    F: std::future::Future<Output = T>,
{
    let (subscriber, bytes) = capturing();
    let output = tracing::instrument::WithSubscriber::with_subscriber(future, subscriber).await;
    let text = String::from_utf8(lock(&bytes).clone()).unwrap();
    (output, text)
}
