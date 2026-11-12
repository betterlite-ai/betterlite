use std::sync::Arc;

use betterlite_context::local_ttl_cache::CacheValue;
use betterlite_redis::redis_cache::RedisCache;

use crate::backend::memory::local::LocalMemoryOps;
use crate::cache_key::value::KeyObject;
use crate::dual_cache::{DualCache, DualCacheSettings};

use super::kwargs::drop_logging_obj_from_kwargs;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OriginalFunction {
    pub name: String,
    pub parameters: Vec<String>,
}

// the cache tier is chosen by the caller: a Redis primary gets a dual cache, every other tier reads its own backend
pub struct HandlerDependencies {
    pub original_function: OriginalFunction,
    pub start_time: f64,
    pub redis_cache: Option<Arc<RedisCache>>,
    pub in_memory_cache: Arc<LocalMemoryOps>,
    pub dual_settings: DualCacheSettings,
}

pub struct CachingHandler {
    pub async_streaming_chunks: Vec<CacheValue>,
    pub sync_streaming_chunks: Vec<CacheValue>,
    pub request_kwargs: KeyObject,
    pub preset_cache_key: Option<String>,
    pub original_function: OriginalFunction,
    pub start_time: f64,
    pub dual_cache: Option<DualCache<LocalMemoryOps>>,
}

impl CachingHandler {
    #[must_use]
    pub fn new(dependencies: HandlerDependencies, request_kwargs: KeyObject) -> Self {
        let HandlerDependencies {
            original_function,
            start_time,
            redis_cache,
            in_memory_cache,
            dual_settings,
        } = dependencies;
        let dual_cache = redis_cache
            .map(|redis| DualCache::new(Arc::clone(&in_memory_cache), Some(redis), dual_settings));
        Self {
            async_streaming_chunks: Vec::new(),
            sync_streaming_chunks: Vec::new(),
            request_kwargs: drop_logging_obj_from_kwargs(request_kwargs),
            preset_cache_key: None,
            original_function,
            start_time,
            dual_cache,
        }
    }
}
