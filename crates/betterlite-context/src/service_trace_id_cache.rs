use std::sync::{Mutex, PoisonError};

use thiserror::Error;

use crate::dynamic_logging_cache::Clock;
use crate::local_ttl_cache::{CacheError, CacheValue, LocalTtlCache};

const ITEM_KIB: i64 = 1024;

#[derive(Debug, Error)]
pub enum ServiceTraceIdCacheError {
    #[error("the service trace id cache rejected the entry")]
    Cache {
        #[from]
        source: CacheError,
    },
}

pub struct ServiceTraceIDCache {
    cache: Mutex<LocalTtlCache>,
}

impl ServiceTraceIDCache {
    pub fn new(clock: Clock) -> Result<Self, ServiceTraceIdCacheError> {
        let cache = LocalTtlCache::with_clock(None, None, None, ITEM_KIB, clock)?;
        Ok(Self {
            cache: Mutex::new(cache),
        })
    }

    pub fn get_cache(
        &self,
        litellm_call_id: &str,
        service_name: &str,
    ) -> Result<Option<String>, ServiceTraceIdCacheError> {
        let key = format!("{service_name}:{litellm_call_id}");
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        let cached = cache.get(&key)?;
        drop(cache);
        let Some(value) = cached.as_ref() else {
            return Ok(None);
        };
        if let CacheValue::Array(values) = value
            && let [CacheValue::Text(trace_id)] = values.as_slice()
        {
            return Ok(Some(trace_id.clone()));
        }
        Ok(None)
    }

    pub fn set_cache(
        &self,
        litellm_call_id: &str,
        service_name: &str,
        trace_id: &str,
    ) -> Result<(), ServiceTraceIdCacheError> {
        let key = format!("{service_name}:{litellm_call_id}");
        // the native cache decodes stored text as JSON, so the id rides in an array to come back unchanged
        let value = CacheValue::Array(vec![CacheValue::Text(trace_id.to_owned())]);
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache.set(&key, value, None)?;
        drop(cache);
        Ok(())
    }
}
