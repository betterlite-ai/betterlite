mod operations;

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use self::operations::increment;
use async_trait::async_trait;
use betterlite_context::local_ttl_cache::{CacheValue, LocalTtlCache};

use super::{BackendError, CacheBackend, CacheResult, Ttl};
use crate::types::pipeline::RedisPipelineIncrement;

pub const DEFAULT_MAX_SIZE_PER_ITEM_KIB: i64 = 1024;

type Clock = Arc<dyn Fn() -> f64 + Send + Sync>;

pub struct MemoryCache {
    values: Mutex<LocalTtlCache>,
    construction: Construction,
    clock: Clock,
    default_ttl_seconds: i64,
}

struct Construction {
    capacity: Option<i64>,
    default_ttl: Option<f64>,
    max_size_per_item: Option<i64>,
    loaded_item_kib: i64,
}

impl MemoryCache {
    pub fn new(
        capacity: Option<i64>,
        default_ttl: Option<f64>,
        max_size_per_item: Option<i64>,
        loaded_item_kib: i64,
        clock: Clock,
    ) -> CacheResult<Self> {
        let values = LocalTtlCache::with_clock(
            capacity,
            default_ttl,
            max_size_per_item,
            loaded_item_kib,
            Arc::clone(&clock),
        )?;
        let default_ttl_seconds = whole_seconds(values.as_ref().default_ttl)?;
        Ok(Self {
            values: Mutex::new(values),
            construction: Construction {
                capacity,
                default_ttl,
                max_size_per_item,
                loaded_item_kib,
            },
            clock,
            default_ttl_seconds,
        })
    }
}

#[async_trait]
impl CacheBackend for MemoryCache {
    fn default_ttl(&self) -> i64 {
        self.default_ttl_seconds
    }

    fn get_cache(&self, key: &str) -> CacheResult<Option<CacheValue>> {
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        values.get(key).map_err(BackendError::from)
    }

    fn set_cache(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        values.set(key, value, ttl).map_err(BackendError::from)
    }

    fn delete_cache(&self, key: &str) -> CacheResult<()> {
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        values.delete(key);
        drop(values);
        Ok(())
    }

    fn flush_cache(&self) -> CacheResult<()> {
        let construction = &self.construction;
        let fresh = LocalTtlCache::with_clock(
            construction.capacity,
            construction.default_ttl,
            construction.max_size_per_item,
            construction.loaded_item_kib,
            Arc::clone(&self.clock),
        )?;
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        *values = fresh;
        drop(values);
        Ok(())
    }

    async fn async_get_cache(&self, key: &str) -> CacheResult<Option<CacheValue>> {
        self.get_cache(key)
    }

    async fn async_set_cache(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        self.set_cache(key, value, ttl)
    }

    async fn async_set_cache_pipeline(
        &self,
        entries: Vec<(String, CacheValue)>,
        ttl: Ttl,
    ) -> CacheResult<()> {
        for (key, value) in entries {
            self.set_cache(&key, value, ttl)?;
        }
        Ok(())
    }

    async fn async_batch_get_cache(&self, keys: &[String]) -> CacheResult<Vec<Option<CacheValue>>> {
        keys.iter().map(|key| self.get_cache(key)).collect()
    }

    async fn async_increment(&self, key: &str, value: f64, ttl: Ttl) -> CacheResult<f64> {
        let mut values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        increment(&mut values, key, value, ttl)
    }

    async fn async_increment_pipeline(
        &self,
        increments: Vec<RedisPipelineIncrement>,
    ) -> CacheResult<Vec<f64>> {
        let mut results = Vec::with_capacity(increments.len());
        for increment in increments {
            results.push(
                self.async_increment(&increment.key, increment.increment_value, None)
                    .await?,
            );
        }
        Ok(results)
    }

    async fn async_get_ttl(&self, key: &str) -> CacheResult<Option<f64>> {
        let values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(values
            .ttl_entries()
            .into_iter()
            .find(|(name, _)| name.as_str() == key)
            .map(|(_, expiry)| expiry))
    }

    async fn async_get_oldest_n_keys(&self, limit: usize) -> CacheResult<Vec<String>> {
        let values = self.values.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(values
            .ttl_entries()
            .into_iter()
            .take(limit)
            .map(|(key, _)| key)
            .collect())
    }

    async fn disconnect(&self) -> CacheResult<()> {
        Ok(())
    }
}

fn whole_seconds(seconds: f64) -> Result<i64, BackendError> {
    let magnitude = Duration::try_from_secs_f64(seconds.abs()).map_err(|source| {
        BackendError::InvalidDefaultDuration {
            value: seconds,
            source,
        }
    })?;
    let whole = i64::try_from(magnitude.as_secs()).map_err(|source| {
        BackendError::InvalidDefaultSeconds {
            value: seconds,
            source,
        }
    })?;
    if seconds.is_sign_negative() {
        0_i64
            .checked_sub(whole)
            .ok_or(BackendError::InvalidDefaultTtl { value: seconds })
    } else {
        Ok(whole)
    }
}
