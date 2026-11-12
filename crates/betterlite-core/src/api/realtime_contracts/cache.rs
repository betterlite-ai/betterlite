use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::{Mutex, PoisonError};
const REQUEST_BODY_CACHE_CAPACITY: NonZeroUsize = match NonZeroUsize::new(1000) {
    Some(capacity) => capacity,
    None => NonZeroUsize::MIN,
};
#[derive(Debug)]
pub struct RealtimeRequestBodyCache {
    entries: Mutex<LruCache<Option<String>, Vec<u8>>>,
}
impl RealtimeRequestBodyCache {
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(LruCache::new(REQUEST_BODY_CACHE_CAPACITY)),
        }
    }
    pub fn body(&self, model: Option<&str>) -> Vec<u8> {
        let key = model.map(str::to_owned);
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(body) = entries.get(&key) {
            return body.clone();
        }
        let body = format!("{{\"model\": \"{}\"}}", model.unwrap_or_default()).into_bytes();
        entries.put(key, body.clone());
        body
    }
    pub fn occupancy(&self) -> CacheOccupancy {
        let entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        CacheOccupancy {
            len: entries.len(),
            capacity: entries.cap().get(),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheOccupancy {
    pub len: usize,
    pub capacity: usize,
}
impl Default for RealtimeRequestBodyCache {
    fn default() -> Self {
        Self::new()
    }
}
