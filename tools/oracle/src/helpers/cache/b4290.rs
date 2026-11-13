#[path = "b4290/cases.rs"]
mod cases;

use std::sync::{Arc, Mutex, PoisonError};

use betterlite_cache::backend::memory::MemoryCache;
use betterlite_cache::backend::memory::local::LocalMemoryOps;
use betterlite_cache::backend::{BASE_DEFAULT_TTL_SECONDS, CacheBackend};
use betterlite_cache::types::control::DynamicCacheControl;
use betterlite_cache::types::embedding::CachedEmbedding;
use betterlite_cache::types::ping::CachePingResponse;
use betterlite_cache::types::pipeline::{
    RedisPipelineIncrement, RedisPipelineLpop, RedisPipelineRpush, RedisPipelineSet,
};
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
        read_at: f64,
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
                read_at,
            } => cases::memory_ttl(*clock, *ttl, key, value, *read_at),
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

const MAX_SIZE_PER_ITEM_KIB: i64 = 1024;

pub(crate) type Now = Arc<Mutex<f64>>;

pub(crate) fn memory(
    start: f64,
) -> Result<(MemoryCache<LocalMemoryOps>, Now), Box<crate::helpers::HelperError>> {
    let now = Arc::new(Mutex::new(start));
    let reader = Arc::clone(&now);
    let operations = LocalMemoryOps::new(
        MAX_SIZE_PER_ITEM_KIB,
        Arc::new(move || *reader.lock().unwrap_or_else(PoisonError::into_inner)),
    )
    .map_err(failure)?;
    Ok((MemoryCache { operations }, now))
}

pub(crate) fn runtime() -> Result<tokio::runtime::Runtime, Box<crate::helpers::HelperError>> {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .map_err(failure)
}
