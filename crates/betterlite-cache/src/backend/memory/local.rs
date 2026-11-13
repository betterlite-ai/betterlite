use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_context::local_ttl_cache::{CacheValue, LocalTtlCache};
use serde_json::Number;

use super::MemoryCacheOps;
use crate::backend::{BackendError, CacheResult, Ttl};
use crate::dual_cache::MemoryMaxSize;
use crate::llm_client_cache::DEFAULT_MAX_SIZE_IN_MEMORY;

type Clock = Arc<dyn Fn() -> f64 + Send + Sync>;

pub struct LocalMemoryOps {
    cache: Mutex<LocalTtlCache>,
    default_ttl_seconds: i64,
    max_size: AtomicUsize,
}

impl LocalMemoryOps {
    // the shared cache gets no size cap of its own: store enforces the live max size, 200 until resized
    pub fn new(max_size_per_item_kib: i64, clock: Clock) -> CacheResult<Self> {
        let cache =
            LocalTtlCache::with_clock(Some(i64::MAX), None, None, max_size_per_item_kib, clock)?;
        let default_ttl = cache.as_ref().default_ttl;
        let whole = Duration::try_from_secs_f64(default_ttl).map_err(|source| {
            BackendError::InvalidDefaultDuration {
                value: default_ttl,
                source,
            }
        })?;
        let default_ttl_seconds = i64::try_from(whole.as_secs()).map_err(|source| {
            BackendError::InvalidDefaultSeconds {
                value: default_ttl,
                source,
            }
        })?;
        Ok(Self {
            cache: Mutex::new(cache),
            default_ttl_seconds,
            max_size: AtomicUsize::new(DEFAULT_MAX_SIZE_IN_MEMORY),
        })
    }

    // LiteLLM's evict_cache: the earliest expiries go until one slot is free, and expired ones sort first
    fn store(
        &self,
        cache: &mut LocalTtlCache,
        key: &str,
        value: CacheValue,
        ttl: Ttl,
    ) -> CacheResult<()> {
        let max_size = self.max_size.load(Ordering::Relaxed);
        if max_size == 0 {
            return Ok(());
        }
        let mut entries = cache.ttl_entries();
        entries.sort_by(|(_first, left), (_second, right)| left.total_cmp(right));
        let surplus = entries.len().saturating_add(1).saturating_sub(max_size);
        for (name, _expiry) in entries.iter().take(surplus) {
            cache.delete(name);
        }
        Ok(cache.set(key, value, ttl)?)
    }
}

impl MemoryMaxSize for LocalMemoryOps {
    fn set_max_size(&self, max_size: usize) {
        self.max_size.store(max_size, Ordering::Relaxed);
    }
}

impl MemoryCacheOps for LocalMemoryOps {
    fn default_ttl(&self) -> i64 {
        self.default_ttl_seconds
    }

    fn get_cache(&self, key: &str) -> CacheResult<Option<CacheValue>> {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(cache.get(key)?)
    }

    fn set_cache(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        self.store(&mut cache, key, value, ttl)
    }

    fn delete_cache(&self, key: &str) -> CacheResult<()> {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache.delete(key);
        drop(cache);
        Ok(())
    }

    fn flush_cache(&self) -> CacheResult<()> {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache.clear();
        drop(cache);
        Ok(())
    }

    fn batch_get_cache(&self, keys: &[String]) -> CacheResult<Vec<Option<CacheValue>>> {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        keys.iter()
            .map(|key| cache.get(key).map_err(BackendError::from))
            .collect()
    }

    // the read and the write share one lock, so a concurrent increment cannot interleave
    fn increment_cache(&self, key: &str, value: f64, ttl: Ttl) -> CacheResult<f64> {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        let base = stored_number(cache.get(key)?.as_ref(), key)?;
        let next = base.mul_add(1.0_f64, value);
        if !next.is_finite() {
            return Err(BackendError::IncrementOutOfRange {
                key: key.to_owned(),
            });
        }
        self.store(&mut cache, key, CacheValue::Float(next), ttl)?;
        drop(cache);
        Ok(next)
    }

    fn ttl_entries(&self) -> CacheResult<Vec<(String, f64)>> {
        let cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(cache.ttl_entries())
    }
}

fn stored_number(stored: Option<&CacheValue>, key: &str) -> CacheResult<f64> {
    match stored {
        None => Ok(0.0_f64),
        Some(CacheValue::Integer(number)) => {
            Number::from(*number)
                .as_f64()
                .ok_or_else(|| BackendError::NotNumeric {
                    key: key.to_owned(),
                })
        }
        Some(CacheValue::Float(number)) => Ok(*number),
        Some(_) => Err(BackendError::NotNumeric {
            key: key.to_owned(),
        }),
    }
}
