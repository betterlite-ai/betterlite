use betterlite_cache::backend::CacheBackend as _;
use betterlite_cache::types::cache_type::LiteLlmCacheType;
use betterlite_cache::types::call_type::SUPPORTED_CALL_TYPES;
use betterlite_cache::types::embedding::{CachedEmbedding, EMBEDDING_CACHE_FORMAT_VERSION};
use betterlite_cache::types::pipeline::RedisPipelineIncrement;
use betterlite_cache::types::semantic::SemanticCacheScope;
use betterlite_context::local_ttl_cache::CacheValue;
use serde_json::json;

use super::{BaseContract, IncrementInput, OldestEntryInput, failure, memory, runtime};
use crate::helpers::HelperResult;

pub(crate) fn base_get() -> HelperResult {
    let value = BaseContract.get_cache("key").map_err(failure)?;
    Ok(json!({"value": value}))
}

pub(crate) fn base_set() -> HelperResult {
    BaseContract
        .set_cache("key", CacheValue::Integer(1), None)
        .map_err(failure)?;
    Ok(json!({"value": null}))
}

pub(crate) fn base_test_connection() -> HelperResult {
    let runtime = runtime()?;
    let outcome = runtime.block_on(async { BaseContract.test_connection().await.map_err(failure) });
    Ok(json!({"status": outcome?}))
}

pub(crate) fn memory_ttl(clock: f64, ttl: f64, key: &str, value: &str) -> HelperResult {
    let cache = memory(clock)?;
    let runtime = runtime()?;
    let outcome: HelperResult = runtime.block_on(async {
        cache
            .async_set_cache(key, CacheValue::Text(value.to_owned()), Some(ttl))
            .await
            .map_err(failure)?;
        let stored = cache.async_get_ttl(key).await.map_err(failure)?;
        Ok(json!({"ttl": stored}))
    });
    outcome
}

pub(crate) fn memory_increments(clock: f64, inputs: &[IncrementInput]) -> HelperResult {
    let records = inputs
        .iter()
        .map(|input| RedisPipelineIncrement {
            key: input.key.clone(),
            increment_value: input.increment_value,
            ttl: None,
        })
        .collect::<Vec<_>>();
    let cache = memory(clock)?;
    let runtime = runtime()?;
    let outcome: HelperResult = runtime.block_on(async {
        let results = cache
            .async_increment_pipeline(records)
            .await
            .map_err(failure)?;
        Ok(json!({"results": results}))
    });
    outcome
}

pub(crate) fn oldest_keys(clock: f64, entries: &[OldestEntryInput], limit: usize) -> HelperResult {
    let cache = memory(clock)?;
    let runtime = runtime()?;
    let outcome: HelperResult = runtime.block_on(async {
        for entry in entries {
            cache
                .async_set_cache(
                    &entry.key,
                    CacheValue::Text(entry.key.clone()),
                    Some(entry.ttl),
                )
                .await
                .map_err(failure)?;
        }
        let keys = cache
            .async_get_oldest_n_keys(limit)
            .await
            .map_err(failure)?;
        Ok(json!({"keys": keys}))
    });
    outcome
}

pub(crate) fn semantic_scopes() -> HelperResult {
    let key = serde_json::to_value(SemanticCacheScope::Key).map_err(failure)?;
    let end_user = serde_json::to_value(SemanticCacheScope::EndUser).map_err(failure)?;
    Ok(json!({"scopes": [key, end_user]}))
}

pub(crate) fn supported_call_types() -> HelperResult {
    let wires = SUPPORTED_CALL_TYPES
        .iter()
        .map(|call_type| serde_json::to_value(call_type).map_err(failure))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({"call_types": wires}))
}

pub(crate) fn cache_types() -> HelperResult {
    let all = [
        LiteLlmCacheType::Local,
        LiteLlmCacheType::Redis,
        LiteLlmCacheType::RedisSemantic,
        LiteLlmCacheType::ValkeySemantic,
        LiteLlmCacheType::S3,
        LiteLlmCacheType::Disk,
        LiteLlmCacheType::QdrantSemantic,
        LiteLlmCacheType::AzureBlob,
        LiteLlmCacheType::Gcs,
    ];
    let mut wires = Vec::with_capacity(all.len());
    for cache_type in all {
        let wire = serde_json::to_value(cache_type).map_err(failure)?;
        let round_trip: LiteLlmCacheType = serde_json::from_value(wire.clone()).map_err(failure)?;
        if round_trip != cache_type {
            return Err(failure("a cache type did not round trip"));
        }
        wires.push(wire);
    }
    Ok(json!({"cache_types": wires}))
}

pub(crate) fn cached_embedding(embedding: &CachedEmbedding) -> HelperResult {
    if embedding.format_version != EMBEDDING_CACHE_FORMAT_VERSION {
        return Err(failure("the cached embedding format version is not 2"));
    }
    Ok(json!({"embedding": embedding}))
}
