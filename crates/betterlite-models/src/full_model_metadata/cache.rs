use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex, PoisonError};

use lru::LruCache;

use crate::model_catalog_keys::ClearableCache;
use crate::model_metadata_resolution::MetadataRequest;

use super::record::ModelInfoHandle;

// The credential-free cache key: api_key is a per-caller credential and never part of it.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct CacheKey {
    pub(crate) model: String,
    pub(crate) custom_llm_provider: Option<String>,
    pub(crate) api_base: Option<String>,
}

impl CacheKey {
    pub(crate) fn for_request(request: &MetadataRequest<'_>) -> Self {
        Self {
            model: request.model.to_owned(),
            custom_llm_provider: request.custom_llm_provider.map(str::to_owned),
            api_base: request.api_base.map(str::to_owned),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CacheInfo {
    pub hits: u64,
    pub misses: u64,
    pub maxsize: usize,
    pub currsize: usize,
}

#[derive(Debug)]
pub struct ModelInfoCache {
    maxsize: usize,
    state: Mutex<CacheState>,
}

#[derive(Debug)]
struct CacheState {
    entries: Option<LruCache<CacheKey, ModelInfoHandle>>,
    hits: u64,
    misses: u64,
}

#[derive(Debug)]
pub struct MetadataCaches {
    pub full: ModelInfoCache,
    pub helper: ModelInfoCache,
}

impl ModelInfoCache {
    // A negative or zero maxsize disables caching, as functools.lru_cache does
    pub fn new(maxsize: i64) -> Self {
        let capacity = usize::try_from(maxsize).unwrap_or(0);
        // sparse keeps the cap without allocating for it, like functools.lru_cache
        let entries = NonZeroUsize::new(capacity).map(LruCache::sparse);
        Self {
            maxsize: capacity,
            state: Mutex::new(CacheState {
                entries,
                hits: 0,
                misses: 0,
            }),
        }
    }

    pub(crate) fn get(&self, key: &CacheKey) -> Option<ModelInfoHandle> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let cached = state
            .entries
            .as_mut()
            .and_then(|cache| cache.get(key))
            .map(Arc::clone);
        if cached.is_some() {
            state.hits = state.hits.saturating_add(1);
        } else {
            state.misses = state.misses.saturating_add(1);
        }
        drop(state);
        cached
    }

    pub(crate) fn put(&self, key: CacheKey, handle: ModelInfoHandle) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(cache) = state.entries.as_mut() {
            drop(cache.put(key, handle));
        }
    }

    // functools.lru_cache.cache_clear resets the statistics along with the entries
    pub fn cache_clear(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(cache) = state.entries.as_mut() {
            cache.clear();
        }
        state.hits = 0;
        state.misses = 0;
    }

    pub fn cache_info(&self) -> CacheInfo {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        CacheInfo {
            hits: state.hits,
            misses: state.misses,
            maxsize: self.maxsize,
            currsize: state.entries.as_ref().map_or(0, LruCache::len),
        }
    }
}

impl ClearableCache for ModelInfoCache {
    fn clear(&mut self) {
        Self::cache_clear(self);
    }
}
