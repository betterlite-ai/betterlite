use std::sync::{Arc, Mutex, PoisonError};

use betterlite_context::local_ttl_cache::{CacheError, CacheValue, LocalTtlCache};

// LiteLLM builds InMemoryCache(max_size_in_memory=10) here: ten entries, the 600-second default TTL and 1024 KiB per item
const CAPACITY: i64 = 10;
const TTL_SECONDS: f64 = 600.0;
const ITEM_KIB: i64 = 1024;
const ITEM_CHARS: usize = 0x0010_0000;

pub struct ImageCache {
    memory: Mutex<LocalTtlCache>,
}

impl ImageCache {
    pub fn new(clock: Arc<dyn Fn() -> f64 + Send + Sync>) -> Result<Self, CacheError> {
        let memory = LocalTtlCache::with_clock(
            Some(CAPACITY),
            Some(TTL_SECONDS),
            Some(ITEM_KIB),
            ITEM_KIB,
            clock,
        )?;
        Ok(Self {
            memory: Mutex::new(memory),
        })
    }

    pub fn get(&self, url: &str) -> Option<String> {
        let mut memory = self.memory.lock().unwrap_or_else(PoisonError::into_inner);
        match memory.get(url) {
            Ok(Some(CacheValue::Bytes(bytes))) => match String::from_utf8(bytes) {
                Ok(text) => Some(text),
                Err(error) => {
                    tracing::warn!(error = %error, "the cached image is not text");
                    None
                }
            },
            Ok(Some(CacheValue::Text(text))) => Some(text),
            Ok(_) => None,
            Err(error) => {
                tracing::warn!(error = %error, "the image cache lookup failed");
                None
            }
        }
    }

    pub fn store(&self, url: &str, data_url: &str) {
        // InMemoryCache.check_value_size keeps a string only below max_size_per_item * 1024 characters
        if data_url.chars().count() >= ITEM_CHARS {
            return;
        }
        let mut memory = self.memory.lock().unwrap_or_else(PoisonError::into_inner);
        let value = CacheValue::Bytes(data_url.as_bytes().to_vec());
        if let Err(error) = memory.set(url, value, None) {
            tracing::warn!(error = %error, "the image cache store failed");
        }
    }
}
