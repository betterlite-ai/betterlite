use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::cache_type::{CacheMode, LiteLlmCacheType};
use super::call_type::CachingSupportedCallType;
use super::control::DynamicCacheControl;
use super::embedding::CachedEmbedding;
use super::ping::{CachePingResponse, HealthCheckCacheParams};
use super::pipeline::{
    RedisPipelineIncrement, RedisPipelineLpop, RedisPipelineRpush, RedisPipelineSet,
};
use super::semantic::SemanticCacheScope;

pub const MAX_CACHE_TYPE_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheTypeRecord {
    CacheType(LiteLlmCacheType),
    CacheMode(CacheMode),
    SemanticScope(SemanticCacheScope),
    CallType(CachingSupportedCallType),
    DynamicCacheControl(DynamicCacheControl),
    CachePingResponse(CachePingResponse),
    HealthCheckCacheParams(HealthCheckCacheParams),
    CachedEmbedding(CachedEmbedding),
    RedisPipelineIncrement(RedisPipelineIncrement),
    RedisPipelineSet(RedisPipelineSet),
    RedisPipelineRpush(RedisPipelineRpush),
    RedisPipelineLpop(RedisPipelineLpop),
}

#[derive(Debug, Error)]
pub enum CacheTypesError {
    #[error("cache type input of {bytes} bytes is past the {limit}-byte limit")]
    TooLarge { bytes: usize, limit: usize },
    #[error("cache type input could not be decoded")]
    Decode {
        #[source]
        source: serde_json::Error,
    },
}

pub fn decode_cache_types(bytes: &[u8]) -> Result<CacheTypeRecord, CacheTypesError> {
    if bytes.len() > MAX_CACHE_TYPE_BYTES {
        return Err(CacheTypesError::TooLarge {
            bytes: bytes.len(),
            limit: MAX_CACHE_TYPE_BYTES,
        });
    }
    serde_json::from_slice(bytes).map_err(|source| CacheTypesError::Decode { source })
}
