use async_trait::async_trait;
use betterlite_models::decisions::Json;

use super::error::CallError;
use super::ports::CallTimes;
use crate::call_setup::RequestMap;
use crate::call_setup::correlation::CallOutcome;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheMode {
    Sync,
    Async,
}

// What _sync_get_cache and _async_get_cache answer with, without the Python objects the wrapper cannot carry
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CacheOutcome {
    pub cached: Option<CallOutcome>,
    pub final_embedding: Option<Json>,
    pub all_embeddings_cached: bool,
    pub uncached_input: Option<Json>,
    pub preset_cache_key: Option<String>,
}

// B4300's CachingHandler, B4303's embedding merge and B4304's streaming wrap, behind one seam
#[async_trait]
pub trait CacheHandler: Send + Sync {
    // litellm.cache is not None, so the sync gate knows whether an absent caching kwarg means yes
    fn configured(&self) -> bool;

    async fn lookup(
        &self,
        request: &RequestMap,
        call_type: &str,
        mode: CacheMode,
    ) -> Result<CacheOutcome, CallError>;

    async fn store(
        &self,
        request: &RequestMap,
        outcome: &CallOutcome,
        call_type: &str,
        mode: CacheMode,
    ) -> Result<(), CallError>;

    fn wrap_streaming_result(&self, outcome: CallOutcome, call_type: &str) -> CallOutcome;

    // _combine_cached_embedding_response_with_api_result; B4303 owns the merge itself
    fn combine_cached_embedding(
        &self,
        final_embedding: &Json,
        fresh: CallOutcome,
        times: CallTimes,
    ) -> CallOutcome;
}

// litellm.cache is None: nothing is read, nothing is stored and a stream passes through
pub struct NoCache;

#[async_trait]
impl CacheHandler for NoCache {
    fn configured(&self) -> bool {
        false
    }

    async fn lookup(
        &self,
        _request: &RequestMap,
        _call_type: &str,
        _mode: CacheMode,
    ) -> Result<CacheOutcome, CallError> {
        Ok(CacheOutcome::default())
    }

    async fn store(
        &self,
        _request: &RequestMap,
        _outcome: &CallOutcome,
        _call_type: &str,
        _mode: CacheMode,
    ) -> Result<(), CallError> {
        Ok(())
    }

    fn wrap_streaming_result(&self, outcome: CallOutcome, _call_type: &str) -> CallOutcome {
        outcome
    }

    fn combine_cached_embedding(
        &self,
        _final_embedding: &Json,
        fresh: CallOutcome,
        _times: CallTimes,
    ) -> CallOutcome {
        fresh
    }
}
