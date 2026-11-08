mod operations;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use self::operations::{increment, oldest_keys, read, whole_seconds, write};
use async_trait::async_trait;
use betterlite_context::local_ttl_cache::{CacheLimits, CacheValue, LocalTtlCache};

use super::{BackendError, CacheBackend};
use crate::types::pipeline::RedisPipelineIncrement;

pub const DEFAULT_CAPACITY: i64 = 200;
pub const DEFAULT_MAX_SIZE_PER_ITEM_KIB: i64 = 1024;

type Clock = Arc<dyn Fn() -> f64 + Send + Sync>;

pub struct MemoryCache {
    state: Mutex<MemoryState>,
    settings: MemorySettings,
}

struct MemorySettings {
    limits: CacheLimits,
    construction: Construction,
    clock: Clock,
    default_ttl_seconds: i64,
}

struct Construction {
    capacity: Option<i64>,
    default_ttl: Option<f64>,
    max_size_per_item: Option<i64>,
    loaded_item_kib: i64,
    item_kib: i64,
}

struct MemoryState {
    values: LocalTtlCache,
    expiries: BTreeMap<String, f64>,
}

impl MemoryCache {
    pub fn new(
        capacity: Option<i64>,
        default_ttl: Option<f64>,
        max_size_per_item: Option<i64>,
        loaded_item_kib: i64,
        clock: Clock,
    ) -> Result<Self, BackendError> {
        let values = LocalTtlCache::with_clock(
            capacity,
            default_ttl,
            max_size_per_item,
            loaded_item_kib,
            Arc::clone(&clock),
        )?;
        let resolved = values.as_ref();
        let limits = CacheLimits {
            capacity: resolved.capacity,
            default_ttl: resolved.default_ttl,
            max_size_per_item: resolved.max_size_per_item,
        };
        let settings = MemorySettings {
            construction: Construction {
                capacity,
                default_ttl,
                max_size_per_item,
                loaded_item_kib,
                item_kib: limits.max_size_per_item,
            },
            default_ttl_seconds: whole_seconds(limits.default_ttl)?,
            limits,
            clock,
        };
        Ok(Self {
            state: Mutex::new(MemoryState {
                values,
                expiries: BTreeMap::new(),
            }),
            settings,
        })
    }
}

#[async_trait]
impl CacheBackend for MemoryCache {
    fn default_ttl(&self) -> i64 {
        self.settings.default_ttl_seconds
    }

    fn get_cache(&self, key: &str) -> Result<Option<CacheValue>, BackendError> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        read(&mut state, key)
    }

    fn set_cache(
        &self,
        key: &str,
        value: CacheValue,
        ttl: Option<f64>,
    ) -> Result<(), BackendError> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        write(&mut state, &self.settings, key, value, ttl)
    }

    fn delete_cache(&self, key: &str) -> Result<(), BackendError> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.values.delete(key);
        state.expiries.remove(key);
        drop(state);
        Ok(())
    }

    fn flush_cache(&self) -> Result<(), BackendError> {
        let construction = &self.settings.construction;
        let values = LocalTtlCache::with_clock(
            construction.capacity,
            construction.default_ttl,
            construction.max_size_per_item,
            construction.loaded_item_kib,
            Arc::clone(&self.settings.clock),
        )?;
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.values = values;
        state.expiries.clear();
        drop(state);
        Ok(())
    }

    async fn async_get_cache(&self, key: &str) -> Result<Option<CacheValue>, BackendError> {
        self.get_cache(key)
    }

    async fn async_set_cache(
        &self,
        key: &str,
        value: CacheValue,
        ttl: Option<f64>,
    ) -> Result<(), BackendError> {
        self.set_cache(key, value, ttl)
    }

    async fn async_set_cache_pipeline(
        &self,
        entries: Vec<(String, CacheValue)>,
        ttl: Option<f64>,
    ) -> Result<(), BackendError> {
        for (key, value) in entries {
            self.set_cache(&key, value, ttl)?;
        }
        Ok(())
    }

    async fn async_batch_get_cache(
        &self,
        keys: &[String],
    ) -> Result<Vec<Option<CacheValue>>, BackendError> {
        keys.iter().map(|key| self.get_cache(key)).collect()
    }

    async fn async_increment(
        &self,
        key: &str,
        value: f64,
        ttl: Option<f64>,
    ) -> Result<f64, BackendError> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        increment(&mut state, &self.settings, key, value, ttl)
    }

    async fn async_increment_pipeline(
        &self,
        increments: Vec<RedisPipelineIncrement>,
    ) -> Result<Vec<f64>, BackendError> {
        let mut results = Vec::with_capacity(increments.len());
        for increment in increments {
            results.push(
                self.async_increment(&increment.key, increment.increment_value, None)
                    .await?,
            );
        }
        Ok(results)
    }

    async fn async_get_ttl(&self, key: &str) -> Result<Option<f64>, BackendError> {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(state.expiries.get(key).copied())
    }

    async fn async_get_oldest_n_keys(&self, limit: usize) -> Result<Vec<String>, BackendError> {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(oldest_keys(&state.expiries, limit))
    }

    async fn disconnect(&self) -> Result<(), BackendError> {
        Ok(())
    }
}
