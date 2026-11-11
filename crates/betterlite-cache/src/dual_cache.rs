mod increments;
pub mod limited_size;
mod operations;
mod reads;
mod value;
mod writes;

#[path = "dual_cache/tests.rs"]
mod tests;

use std::fmt::Display;
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard};

use betterlite_config::env_integer::get_env_int_in_range;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_redis::batch::result::BatchResult;
use betterlite_redis::redis_cache::RedisCache;
use betterlite_redis::redis_cache::error::RedisCacheError;
use betterlite_redis::redis_cache_admin::RedisAdminError;
use betterlite_redis::redis_cache_values::error::RedisCacheValuesError;
use betterlite_redis::redis_counters::RedisCountersError;
use serde::Serialize;
use thiserror::Error;

use self::limited_size::LimitedSizeOrderedDict;
use crate::backend::BackendError;
use crate::backend::memory::MemoryCacheOps;

pub const MAX_REDIS_BATCH_CACHE_SIZE: usize = 1000;
pub const BATCH_CACHE_SIZE_VARIABLE: &str = "DEFAULT_MAX_REDIS_BATCH_CACHE_SIZE";
// DualCache.__init__'s fallback when litellm.default_redis_batch_cache_expiry is unset
pub const DEFAULT_BATCH_CACHE_EXPIRY: f64 = 10.0;
// litellm.caching.in_memory_cache.DEFAULT_MAX_SIZE_IN_MEMORY
pub const DEFAULT_MAX_SIZE_IN_MEMORY: usize = 200;

pub type Deferred = Result<Option<BatchResult<()>>, DualCacheError>;

#[derive(Debug, Error)]
pub enum DualCacheError {
    #[error("the in-memory cache tier failed")]
    Memory(#[from] BackendError),
    #[error("the Redis increment failed")]
    RedisIncrement(#[from] RedisCountersError),
    #[error("the Redis ttl read failed")]
    RedisTtl(#[from] RedisCacheError),
    #[error("the Redis set add failed")]
    RedisSetAdd(#[from] RedisCacheValuesError),
    #[error("the Redis delete failed")]
    RedisDelete(#[from] RedisAdminError),
    #[error("the increment of {value} is not a whole number Redis can add")]
    FractionalIncrement { value: f64 },
    #[error("the Redis count of {value} has no finite float form")]
    CountOutOfRange { value: i64 },
    #[error("this cache value has no JSON representation")]
    UnrepresentableValue,
    #[error("the value under {key} is not a set of strings")]
    NotASet { key: String },
}

#[derive(Clone, Copy, Debug)]
pub struct DualCacheSettings {
    pub default_in_memory_ttl: Option<f64>,
    pub default_redis_ttl: Option<f64>,
    pub batch_cache_expiry: Option<f64>,
    pub max_redis_batch_cache_size: usize,
    pub redis_delete_chunk_size: usize,
}

impl DualCacheSettings {
    pub fn from_environment<Lookup>(lookup: &Lookup) -> Self
    where
        Lookup: EnvironmentLookup,
    {
        let raw = lookup.lookup(BATCH_CACHE_SIZE_VARIABLE);
        let parsed =
            get_env_int_in_range(BATCH_CACHE_SIZE_VARIABLE, raw.as_deref(), 1000, 1, i64::MAX);
        let size = usize::try_from(parsed).unwrap_or(MAX_REDIS_BATCH_CACHE_SIZE);
        Self {
            default_in_memory_ttl: None,
            default_redis_ttl: None,
            batch_cache_expiry: None,
            max_redis_batch_cache_size: size,
            redis_delete_chunk_size: size,
        }
    }
}

// DualCache.async_get_ttl answers the memory tier's float expiry or Redis's whole seconds left
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Ttl {
    Expiry(f64),
    Seconds(i64),
}

#[derive(Clone, Copy, Debug, Default)]
pub struct CacheOptions {
    pub ttl: Option<f64>,
    pub local_only: bool,
    pub refresh_ttl: bool,
}

// InMemoryCache.max_size_in_memory is a plain attribute in LiteLLM; a tier that can resize implements this
pub trait MemoryMaxSize {
    fn set_max_size(&self, max_size: usize);
}

pub struct DualCache<Memory: MemoryCacheOps> {
    pub in_memory_cache: Arc<Memory>,
    pub redis_cache: RwLock<Option<Arc<RedisCache>>>,
    pub last_redis_batch_access_time: Mutex<LimitedSizeOrderedDict<f64>>,
    pub redis_batch_cache_expiry: f64,
    pub redis_delete_chunk_size: usize,
    pub default_in_memory_ttl: RwLock<Option<f64>>,
    pub default_redis_ttl: RwLock<Option<f64>>,
}

pub(crate) fn lock_read<T>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    match lock.read() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

pub(crate) fn lock_write<T>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    match lock.write() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

// LiteLLM logs a cache tier failure and carries on with no value, as every dual-cache read does
pub(crate) fn logged<T, Failure>(result: Result<T, Failure>, message: &str) -> Option<T>
where
    Failure: Display,
{
    match result {
        Ok(value) => Some(value),
        Err(error) => {
            tracing::error!(%error, "{message}");
            None
        }
    }
}
