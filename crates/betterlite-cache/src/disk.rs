use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use betterlite_context::local_ttl_cache::CacheValue;
use thiserror::Error;
use tokio::task::{JoinError, JoinSet};

use crate::backend::Ttl;
use crate::coordinator::AsyncCacheProtocol;

mod store;
use store::DiskStore;

#[path = "disk/tests.rs"]
mod disk_cache_tests;

pub const DEFAULT_DISK_CACHE_DIRECTORY: &str = ".litellm_cache";

// diskcache opens its sqlite connection with a sixty second lock timeout
const LOCK_TIMEOUT: Duration = Duration::from_secs(60);

pub type Clock = Arc<dyn Fn() -> f64 + Send + Sync>;

#[derive(Debug, Error)]
pub enum DiskError {
    #[error("the disk cache directory {path} cannot be used")]
    Directory {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("the disk cache store failed")]
    Store {
        #[source]
        source: rusqlite::Error,
    },
    #[error("the disk cache lock is poisoned")]
    Unavailable,
    #[error("the value under {key} cannot be stored as json")]
    Serialize {
        key: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("the value stored under {key} is not valid json")]
    Decode {
        key: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("the increment under {key} left the whole number range")]
    IncrementOutOfRange { key: String },
    #[error("the disk cache clock is not finite")]
    InvalidClock,
    #[error("the disk cache blocking task failed")]
    Blocking {
        #[from]
        source: JoinError,
    },
    #[error("the disk cache blocking task ended without a result")]
    BlockingMissing,
}

pub type DiskResult<Value> = Result<Value, DiskError>;

pub struct DiskCache {
    store: Arc<DiskStore>,
    clock: Clock,
}

impl DiskCache {
    // an absent directory takes LiteLLM's own .litellm_cache default
    pub fn open(directory: Option<&Path>, clock: Clock) -> Result<Self, DiskError> {
        let path = directory.unwrap_or_else(|| Path::new(DEFAULT_DISK_CACHE_DIRECTORY));
        let store = DiskStore::open(path)?;
        Ok(Self {
            store: Arc::new(store),
            clock,
        })
    }

    fn now(&self) -> Result<f64, DiskError> {
        let now = (self.clock)();
        if now.is_finite() {
            Ok(now)
        } else {
            Err(DiskError::InvalidClock)
        }
    }

    // the sync calls block the calling thread, and the async twins run them on the blocking pool
    pub fn set_cache(&self, key: &str, value: &CacheValue, ttl: Ttl) -> DiskResult<()> {
        let now = self.now()?;
        let expire_at = ttl.map(|seconds| seconds.mul_add(1.0_f64, now));
        self.store.set(key, value, expire_at)
    }

    pub fn get_cache(&self, key: &str) -> DiskResult<Option<CacheValue>> {
        let now = self.now()?;
        let stored = self.store.get(key, now)?;
        Ok(stored.filter(carries_value))
    }

    pub fn batch_get_cache(&self, keys: &[String]) -> DiskResult<Vec<Option<CacheValue>>> {
        let mut values = Vec::with_capacity(keys.len());
        for key in keys {
            values.push(self.get_cache(key)?);
        }
        Ok(values)
    }

    pub fn increment_cache(&self, key: &str, value: i64, ttl: Ttl) -> DiskResult<i64> {
        let now = self.now()?;
        let expire_at = ttl.map(|seconds| seconds.mul_add(1.0_f64, now));
        self.store.increment(key, value, now, expire_at)
    }

    pub fn delete_cache(&self, key: &str) -> DiskResult<()> {
        let now = self.now()?;
        self.store.delete(key, now)?;
        Ok(())
    }

    pub fn flush_cache(&self) -> DiskResult<()> {
        self.store.clear()?;
        Ok(())
    }

    pub async fn async_batch_get_cache(
        &self,
        keys: &[String],
    ) -> DiskResult<Vec<Option<CacheValue>>> {
        let store = Arc::clone(&self.store);
        let now = self.now()?;
        let cache_keys = keys.to_vec();
        let values = run_blocking(move || {
            let mut values = Vec::with_capacity(cache_keys.len());
            for key in &cache_keys {
                values.push(store.get(key, now)?.filter(carries_value));
            }
            Ok(values)
        })
        .await?;
        Ok(values)
    }

    pub async fn async_increment(&self, key: &str, value: i64, ttl: Ttl) -> DiskResult<i64> {
        let next = self
            .run_keyed(key, move |store, cache_key, now| {
                let expire_at = ttl.map(|seconds| seconds.mul_add(1.0_f64, now));
                store.increment(cache_key, value, now, expire_at)
            })
            .await?;
        Ok(next)
    }

    pub async fn async_set_cache_pipeline(
        &self,
        entries: Vec<(String, CacheValue)>,
        ttl: Ttl,
    ) -> DiskResult<()> {
        for (key, value) in entries {
            AsyncCacheProtocol::async_set_cache(self, &key, value, ttl).await?;
        }
        Ok(())
    }

    pub async fn async_delete_cache(&self, key: &str) -> DiskResult<()> {
        self.run_keyed(key, |store, cache_key, now| {
            store.delete(cache_key, now)?;
            Ok(())
        })
        .await
    }

    pub async fn async_flush_cache(&self) -> DiskResult<()> {
        let store = Arc::clone(&self.store);
        run_blocking(move || store.clear()).await?;
        Ok(())
    }

    async fn run_keyed<Value, Operation>(
        &self,
        key: &str,
        operation: Operation,
    ) -> DiskResult<Value>
    where
        Value: Send + 'static,
        Operation: FnOnce(&DiskStore, &str, f64) -> Result<Value, DiskError> + Send + 'static,
    {
        let store = Arc::clone(&self.store);
        let now = self.now()?;
        let cache_key = key.to_owned();
        run_blocking(move || operation(store.as_ref(), &cache_key, now)).await
    }
}

#[async_trait]
impl AsyncCacheProtocol for DiskCache {
    type Error = DiskError;

    async fn async_get_cache(&self, key: &str) -> DiskResult<Option<CacheValue>> {
        self.run_keyed(key, |store, cache_key, now| {
            store
                .get(cache_key, now)
                .map(|stored| stored.filter(carries_value))
        })
        .await
    }

    async fn async_set_cache(&self, key: &str, value: CacheValue, ttl: Ttl) -> DiskResult<()> {
        self.run_keyed(key, move |store, cache_key, now| {
            let expire_at = ttl.map(|seconds| seconds.mul_add(1.0_f64, now));
            store.set(cache_key, &value, expire_at)
        })
        .await
    }
}

// LiteLLM returns None for any stored payload Python reads as false, so a cached zero or empty value is a miss
fn carries_value(value: &CacheValue) -> bool {
    match value {
        CacheValue::Null => false,
        CacheValue::Boolean(flag) => *flag,
        CacheValue::Integer(number) => *number != 0,
        CacheValue::Float(number) => *number != 0.0_f64,
        CacheValue::Text(text) => !text.is_empty(),
        CacheValue::Array(items) => !items.is_empty(),
        CacheValue::Object(entries) => !entries.is_empty(),
        CacheValue::Bytes(bytes) => !bytes.is_empty(),
        CacheValue::Credential(_) => true,
    }
}

async fn run_blocking<Value, Operation>(operation: Operation) -> DiskResult<Value>
where
    Value: Send + 'static,
    Operation: FnOnce() -> Result<Value, DiskError> + Send + 'static,
{
    let mut tasks = JoinSet::new();
    tasks.spawn_blocking(operation);
    let joined = tasks.join_next().await.ok_or(DiskError::BlockingMissing)?;
    joined?
}
