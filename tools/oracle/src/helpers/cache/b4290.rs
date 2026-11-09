#[path = "b4290/cases.rs"]
mod cases;

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};

use betterlite_cache::backend::memory::{MemoryCache, MemoryCacheOps};
use betterlite_cache::backend::{BASE_DEFAULT_TTL_SECONDS, CacheBackend, CacheResult, Ttl};
use betterlite_cache::types::control::DynamicCacheControl;
use betterlite_cache::types::embedding::CachedEmbedding;
use betterlite_cache::types::ping::CachePingResponse;
use betterlite_cache::types::pipeline::{
    RedisPipelineIncrement, RedisPipelineLpop, RedisPipelineRpush, RedisPipelineSet,
};
use betterlite_context::local_ttl_cache::CacheValue;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Input {
    BaseCacheContractGet,
    BaseCacheContractSet,
    #[serde(rename = "base_cache_test_connection_unsupported")]
    BaseTestConnection,
    MemoryAsyncGetAbsoluteTtl {
        clock: f64,
        ttl: f64,
        key: String,
        value: String,
    },
    MemoryAsyncIncrementPipeline {
        clock: f64,
        increments: Vec<IncrementInput>,
    },
    DynamicCacheControlTtl {
        cache: DynamicCacheControl,
    },
    SemanticScopeKeyAndUser,
    #[serde(rename = "cached_embedding_format_version_2")]
    CachedEmbeddingV2 {
        embedding: CachedEmbedding,
    },
    SupportedCallTypes,
    CacheTypeRoundTrip,
    PingResponseHealthCheck {
        ping: CachePingResponse,
    },
    RedisPipelineIncrement {
        operation: RedisPipelineIncrement,
    },
    RedisPipelineSet {
        operation: RedisPipelineSet,
    },
    RedisPipelineRpush {
        operation: RedisPipelineRpush,
    },
    RedisPipelineLpop {
        operation: RedisPipelineLpop,
    },
    AsyncGetOldestNKeysOrdering {
        clock: f64,
        entries: Vec<OldestEntryInput>,
        limit: usize,
    },
}

#[derive(Deserialize)]
pub(crate) struct IncrementInput {
    pub(crate) key: String,
    pub(crate) increment_value: f64,
}

#[derive(Deserialize)]
pub(crate) struct OldestEntryInput {
    pub(crate) key: String,
    pub(crate) ttl: f64,
}

pub(crate) struct BaseContract;

impl CacheBackend for BaseContract {
    fn default_ttl(&self) -> i64 {
        BASE_DEFAULT_TTL_SECONDS
    }
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        match input {
            Input::BaseCacheContractGet => cases::base_get(),
            Input::BaseCacheContractSet => cases::base_set(),
            Input::BaseTestConnection => cases::base_test_connection(),
            Input::MemoryAsyncGetAbsoluteTtl {
                clock,
                ttl,
                key,
                value,
            } => cases::memory_ttl(*clock, *ttl, key, value),
            Input::MemoryAsyncIncrementPipeline { clock, increments } => {
                cases::memory_increments(*clock, increments)
            }
            Input::DynamicCacheControlTtl { cache } => Ok(json!({"cache": cache})),
            Input::SemanticScopeKeyAndUser => cases::semantic_scopes(),
            Input::CachedEmbeddingV2 { embedding } => cases::cached_embedding(embedding),
            Input::SupportedCallTypes => cases::supported_call_types(),
            Input::CacheTypeRoundTrip => cases::cache_types(),
            Input::PingResponseHealthCheck { ping } => Ok(json!({"ping": ping})),
            Input::RedisPipelineIncrement { operation } => Ok(json!({"operation": operation})),
            Input::RedisPipelineSet { operation } => Ok(json!({"operation": operation})),
            Input::RedisPipelineRpush { operation } => Ok(json!({"operation": operation})),
            Input::RedisPipelineLpop { operation } => Ok(json!({"operation": operation})),
            Input::AsyncGetOldestNKeysOrdering {
                clock,
                entries,
                limit,
            } => cases::oldest_keys(*clock, entries, *limit),
        }
    }
}

pub(crate) fn failure<Error>(error: Error) -> Box<crate::helpers::HelperError>
where
    Error: std::fmt::Display,
{
    Box::new(crate::helpers::HelperError {
        class: "ValueError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}

const MEMORY_DEFAULT_TTL: f64 = 600.0;
const MEMORY_DEFAULT_TTL_SECONDS: i64 = 600;

struct FixtureState {
    clock: f64,
    values: BTreeMap<String, CacheValue>,
    counters: BTreeMap<String, f64>,
    entries: Vec<(String, f64)>,
}

pub(crate) struct MemoryFixture {
    state: Mutex<FixtureState>,
}

impl MemoryFixture {
    const fn with_clock(clock: f64) -> Self {
        Self {
            state: Mutex::new(FixtureState {
                clock,
                values: BTreeMap::new(),
                counters: BTreeMap::new(),
                entries: Vec::new(),
            }),
        }
    }

    fn remember(&self, key: &str, ttl: Ttl) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let expiry = state
            .clock
            .mul_add(1.0_f64, ttl.unwrap_or(MEMORY_DEFAULT_TTL));
        match state.entries.iter_mut().find(|(name, _)| name == key) {
            Some(entry) => entry.1 = expiry,
            None => state.entries.push((key.to_owned(), expiry)),
        }
    }
}

impl MemoryCacheOps for MemoryFixture {
    fn default_ttl(&self) -> i64 {
        MEMORY_DEFAULT_TTL_SECONDS
    }

    fn get_cache(&self, key: &str) -> CacheResult<Option<CacheValue>> {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(state.values.get(key).cloned())
    }

    fn set_cache(&self, key: &str, value: CacheValue, ttl: Ttl) -> CacheResult<()> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.values.insert(key.to_owned(), value);
        drop(state);
        self.remember(key, ttl);
        Ok(())
    }

    fn delete_cache(&self, key: &str) -> CacheResult<()> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.values.remove(key);
        state.counters.remove(key);
        state.entries.retain(|(name, _)| name != key);
        drop(state);
        Ok(())
    }

    fn flush_cache(&self) -> CacheResult<()> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.values.clear();
        state.counters.clear();
        state.entries.clear();
        drop(state);
        Ok(())
    }

    fn batch_get_cache(&self, keys: &[String]) -> CacheResult<Vec<Option<CacheValue>>> {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(keys
            .iter()
            .map(|key| state.values.get(key).cloned())
            .collect())
    }

    fn increment_cache(&self, key: &str, value: f64, ttl: Ttl) -> CacheResult<f64> {
        let next = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            let base = state.counters.get(key).copied().unwrap_or(0.0_f64);
            let next = base.mul_add(1.0_f64, value);
            state.counters.insert(key.to_owned(), next);
            state.values.insert(key.to_owned(), CacheValue::Float(next));
            next
        };
        self.remember(key, ttl);
        Ok(next)
    }

    fn ttl_entries(&self) -> CacheResult<Vec<(String, f64)>> {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(state.entries.clone())
    }
}

pub(crate) const fn memory(clock: f64) -> MemoryCache<MemoryFixture> {
    MemoryCache {
        operations: MemoryFixture::with_clock(clock),
    }
}

pub(crate) fn runtime() -> Result<tokio::runtime::Runtime, Box<crate::helpers::HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .map_err(failure)
}
